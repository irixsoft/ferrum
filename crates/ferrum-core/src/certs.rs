use crate::acme::{self, AcmeError, Certificate, Directory, Issuer};
use crate::apps::{self, App, provision};
use crate::dns::{self, Lookup, Verdict};
use crate::dns_providers;
use crate::events::{self, Kind};
use crate::state::State;
use crate::{CERTS_DIR, setup};
use anyhow::Context;
use ferrum_platform::ubuntu::NGINX_UNIT;
use ferrum_platform::{Platform, ServiceAction};
use serde::Serialize;
use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use time::OffsetDateTime;

pub const SWEEP_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub const FIRST_SWEEP: Duration = Duration::from_secs(30);
const DNS_RETRY_SECS: i64 = 5 * 60;
const MAX_ATTEMPTS: i64 = 5;
const GIVE_UP_SECS: i64 = 24 * 60 * 60;
const BASE_BACKOFF_SECS: i64 = 30;
const ARI_REFRESH: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone)]
pub struct Issuance {
    pub directory: Directory,
    pub resolver: Lookup,
    pub http: reqwest::Client,
    public_ip: Arc<Mutex<Option<IpAddr>>>,
    ari_checked: Arc<Mutex<HashMap<String, Instant>>>,
}

impl Issuance {
    pub fn new(directory: Directory, resolver: Lookup, public_ip: Option<IpAddr>) -> Self {
        Self {
            directory,
            resolver,
            http: crate::http::client(),
            public_ip: Arc::new(Mutex::new(public_ip)),
            ari_checked: Arc::default(),
        }
    }

    /// Whether a DNS-01 record must reach the public authoritatives before the CA looks.
    pub fn checks_dns(&self) -> bool {
        !self.directory.is_custom()
    }

    /// True at most once per `ARI_REFRESH` for each domain, and marks it checked.
    fn take_ari_turn(&self, domain: &str) -> bool {
        let mut checked = self.ari_checked.lock().expect("not poisoned");
        match checked.get(domain) {
            Some(at) if at.elapsed() < ARI_REFRESH => false,
            _ => {
                checked.insert(domain.to_string(), Instant::now());
                true
            }
        }
    }

    pub async fn expected_ip(&self) -> anyhow::Result<IpAddr> {
        if let Some(ip) = *self.public_ip.lock().expect("not poisoned") {
            return Ok(ip);
        }
        let ip = dns::public_ip().await?;
        *self.public_ip.lock().expect("not poisoned") = Some(ip);
        Ok(ip)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CertStatus {
    Issued { not_after: String },
    WaitingForDns { detail: String },
    Failed { detail: String, retry_at: String },
    None,
}

#[derive(Debug, Clone, Serialize)]
pub struct DomainCert {
    pub domain: String,
    pub status: CertStatus,
}

struct Attempt {
    attempts: i64,
    last_error: Option<String>,
    next_at: Option<String>,
    waiting: bool,
}

pub fn backoff_secs(attempts: i64) -> i64 {
    if attempts >= MAX_ATTEMPTS {
        GIVE_UP_SECS
    } else {
        BASE_BACKOFF_SECS << attempts
    }
}

fn has_certificate(platform: &dyn Platform, domain: &str) -> bool {
    platform.file_exists(&acme::cert_dir(domain).join("fullchain.pem"))
}

fn not_after(platform: &dyn Platform, domain: &str) -> Option<String> {
    let pem = platform
        .read_file(&acme::cert_dir(domain).join("fullchain.pem"))
        .ok()??;
    let stamp = acme::not_after_of(&pem).ok()?.unix_timestamp();
    chrono::DateTime::from_timestamp(stamp, 0)
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

async fn attempt(state: &State, domain: &str) -> anyhow::Result<Option<Attempt>> {
    let row = sqlx::query!(
        r#"SELECT attempts AS "attempts!", last_error, next_at,
                  (next_at IS NOT NULL AND next_at > datetime('now')) AS "waiting!: bool"
           FROM cert_attempts WHERE domain = ?"#,
        domain
    )
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|r| Attempt {
        attempts: r.attempts,
        last_error: r.last_error,
        next_at: r.next_at,
        waiting: r.waiting,
    }))
}

async fn record(
    state: &State,
    domain: &str,
    attempts: i64,
    error: &str,
    retry_in_secs: i64,
) -> anyhow::Result<()> {
    let offset = format!("+{retry_in_secs} seconds");
    sqlx::query!(
        "INSERT INTO cert_attempts (domain, attempts, last_error, next_at)
         VALUES (?, ?, ?, datetime('now', ?))
         ON CONFLICT(domain) DO UPDATE SET attempts = excluded.attempts,
             last_error = excluded.last_error, next_at = excluded.next_at",
        domain,
        attempts,
        error,
        offset
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Forgets the backoff; the CA's renewal window stays.
async fn clear(state: &State, domain: &str) -> anyhow::Result<()> {
    sqlx::query!(
        "DELETE FROM cert_attempts WHERE domain = ? AND renew_after IS NULL",
        domain
    )
    .execute(&state.pool)
    .await?;
    sqlx::query!(
        "UPDATE cert_attempts SET attempts = 0, last_error = NULL, next_at = NULL WHERE domain = ?",
        domain
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn set_renew_after(
    state: &State,
    domain: &str,
    at: Option<OffsetDateTime>,
) -> anyhow::Result<()> {
    let stamp = at
        .and_then(|t| chrono::DateTime::from_timestamp(t.unix_timestamp(), 0))
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    sqlx::query!(
        "INSERT INTO cert_attempts (domain, attempts, renew_after) VALUES (?, 0, ?)
         ON CONFLICT(domain) DO UPDATE SET renew_after = excluded.renew_after",
        domain,
        stamp
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn renew_after(state: &State, domain: &str) -> anyhow::Result<Option<OffsetDateTime>> {
    let stamp = sqlx::query_scalar!(
        "SELECT renew_after FROM cert_attempts WHERE domain = ?",
        domain
    )
    .fetch_optional(&state.pool)
    .await?
    .flatten();
    Ok(stamp
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
        .and_then(|t| OffsetDateTime::from_unix_timestamp(t.timestamp()).ok()))
}

async fn provider_of(state: &State, domain: &str) -> anyhow::Result<Option<String>> {
    Ok(sqlx::query_scalar!(
        "SELECT dns_provider_id FROM app_domains WHERE domain = ?",
        domain
    )
    .fetch_optional(&state.pool)
    .await?
    .flatten())
}

async fn owner_of(state: &State, domain: &str) -> anyhow::Result<Option<(String, String)>> {
    let row = sqlx::query!(
        r#"SELECT a.id AS "id!", a.slug AS "slug!" FROM app_domains d
           JOIN apps a ON a.id = d.app_id WHERE d.domain = ?"#,
        domain
    )
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|r| (r.id, r.slug)))
}

/// Records the failure; a renewal that has used up its quick retries also tells the owner,
/// which the day-long backoff after that limits to once a day.
async fn failed(
    state: &State,
    domain: &str,
    attempts: i64,
    error: &str,
    renewing: bool,
) -> anyhow::Result<()> {
    record(state, domain, attempts, error, backoff_secs(attempts)).await?;
    if renewing && attempts >= MAX_ATTEMPTS {
        let sentence = format!("The certificate for {domain} could not be renewed: {error}");
        let owner = owner_of(state, domain).await?;
        let link = owner.as_ref().map(|(_, slug)| format!("/apps/{slug}"));
        events::emit(
            state,
            Kind::BrokeOnItsOwn,
            owner.as_ref().map(|(id, _)| id.as_str()),
            domain,
            &sentence,
            link.as_deref(),
        )
        .await;
    }
    Ok(())
}

async fn finish(
    state: &State,
    domain: &str,
    attempts: i64,
    issued: Result<Certificate, AcmeError>,
    renewing: bool,
) -> anyhow::Result<bool> {
    match issued {
        Ok(cert) => {
            clear(state, domain).await?;
            set_renew_after(state, domain, cert.renew_after).await?;
            tracing::info!(domain, "certificate issued");
            Ok(true)
        }
        Err(e) => {
            let attempts = attempts + 1;
            tracing::warn!(domain, attempts, error = %e, "certificate issuance failed");
            failed(state, domain, attempts, &e.to_string(), renewing).await?;
            Ok(false)
        }
    }
}

pub async fn status(
    state: &State,
    platform: &dyn Platform,
    domain: &str,
) -> anyhow::Result<CertStatus> {
    if let Some(not_after) = not_after(platform, domain) {
        return Ok(CertStatus::Issued { not_after });
    }
    Ok(match attempt(state, domain).await? {
        Some(a) if a.attempts > 0 => CertStatus::Failed {
            detail: a.last_error.unwrap_or_default(),
            retry_at: crate::time::utc(a.next_at.unwrap_or_default()),
        },
        Some(Attempt {
            last_error: Some(detail),
            ..
        }) => CertStatus::WaitingForDns { detail },
        _ => CertStatus::None,
    })
}

pub async fn statuses(
    state: &State,
    platform: &dyn Platform,
    app: &App,
) -> anyhow::Result<Vec<DomainCert>> {
    let mut out = Vec::with_capacity(app.domains.len());
    for d in &app.domains {
        out.push(DomainCert {
            domain: d.domain.clone(),
            status: status(state, platform, &d.domain).await?,
        });
    }
    Ok(out)
}

/// Clears the backoff so the next sweep tries at once.
pub async fn retry_now(state: &State, app: &App) -> anyhow::Result<()> {
    for d in &app.domains {
        clear(state, &d.domain).await?;
    }
    Ok(())
}

/// DNS is checked here, locally, so an unpropagated record never counts against Let's Encrypt.
async fn try_issue(
    state: &State,
    platform: &dyn Platform,
    issuance: &Issuance,
    domain: &str,
    renewing: bool,
) -> anyhow::Result<bool> {
    if !renewing && has_certificate(platform, domain) {
        return Ok(false);
    }
    let previous = attempt(state, domain).await?;
    if previous.as_ref().is_some_and(|a| a.waiting) {
        return Ok(false);
    }
    let attempts = previous.map(|a| a.attempts).unwrap_or(0);
    let expected = match issuance.expected_ip().await {
        Ok(ip) => ip,
        Err(e) => {
            record(state, domain, attempts, &format!("{e:#}"), DNS_RETRY_SECS).await?;
            return Ok(false);
        }
    };
    let verdict = match issuance.resolver.verify(domain, expected).await {
        Ok(v) => v,
        Err(e) => {
            record(state, domain, attempts, &e.to_string(), DNS_RETRY_SECS).await?;
            return Ok(false);
        }
    };
    if verdict != Verdict::Match {
        record(
            state,
            domain,
            attempts,
            &dns::describe(&verdict, domain),
            DNS_RETRY_SECS,
        )
        .await?;
        return Ok(false);
    }
    let email = setup::email(state)
        .await?
        .context("no contact email is set for certificates")?;
    let issued = match Issuer::new(state, issuance.directory.clone(), &email).await {
        Ok(issuer) => issuer.issue(domain, &acme::cert_dir(domain)).await,
        Err(e) => Err(e),
    };
    finish(state, domain, attempts, issued, renewing).await
}

/// `name` is `*.<base>`, proven over DNS-01 through the provider, so no A record is needed.
/// The caller brings the site in line afterwards.
pub async fn issue_wildcard(
    state: &State,
    platform: &dyn Platform,
    issuance: &Issuance,
    name: &str,
    provider_id: &str,
    renewing: bool,
) -> anyhow::Result<bool> {
    if !renewing && has_certificate(platform, name) {
        return Ok(false);
    }
    let previous = attempt(state, name).await?;
    if previous.as_ref().is_some_and(|a| a.waiting) {
        return Ok(false);
    }
    let attempts = previous.map(|a| a.attempts).unwrap_or(0);
    let email = setup::email(state)
        .await?
        .context("no contact email is set for certificates")?;
    let issued = async {
        let provider = dns_providers::client(state, &issuance.http, provider_id)
            .await
            .map_err(|e| AcmeError::Dns(format!("{e:#}")))?;
        let issuer = Issuer::new(state, issuance.directory.clone(), &email).await?;
        issuer
            .issue_dns(
                name,
                &acme::cert_dir(name),
                provider.as_ref(),
                issuance.checks_dns(),
            )
            .await
    }
    .await;
    finish(state, name, attempts, issued, renewing).await
}

async fn issue_one(
    state: &State,
    platform: &dyn Platform,
    issuance: &Issuance,
    domain: &str,
    renewing: bool,
) -> anyhow::Result<bool> {
    if !apps::domains::is_wildcard(domain) {
        return try_issue(state, platform, issuance, domain, renewing).await;
    }
    match provider_of(state, domain).await? {
        Some(id) => issue_wildcard(state, platform, issuance, domain, &id, renewing).await,
        None => {
            if !has_certificate(platform, domain) {
                let attempts = attempt(state, domain)
                    .await?
                    .map(|a| a.attempts)
                    .unwrap_or(0);
                let why = format!(
                    "{domain} is a wildcard, proven through DNS; choose a DNS provider for it."
                );
                record(state, domain, attempts, &why, DNS_RETRY_SECS).await?;
            }
            Ok(false)
        }
    }
}

/// Issues for every domain of the app that has no certificate, then brings the site in line
/// with the certificates on disk; `true` when either changed something.
pub async fn issue_for(
    state: &State,
    platform: &dyn Platform,
    issuance: &Issuance,
    app: &App,
) -> anyhow::Result<bool> {
    let mut landed = false;
    for d in &app.domains {
        landed |= issue_one(state, platform, issuance, &d.domain, false).await?;
    }
    let refreshed = provision::refresh_vhost(platform, app)?;
    Ok(landed || refreshed)
}

struct OnDisk {
    domain: String,
    pem: String,
    not_before: OffsetDateTime,
    not_after: OffsetDateTime,
}

/// Asks the CA for a fresh renewal window, at most every six hours per certificate.
async fn refresh_windows(state: &State, issuance: &Issuance, certs: &[OnDisk]) {
    let due: Vec<&OnDisk> = certs
        .iter()
        .filter(|c| acme::cert_id_of(&c.pem).is_ok())
        .filter(|c| issuance.take_ari_turn(&c.domain))
        .collect();
    if due.is_empty() {
        return;
    }
    let Ok(Some(email)) = setup::email(state).await else {
        return;
    };
    let issuer = match Issuer::new(state, issuance.directory.clone(), &email).await {
        Ok(issuer) => issuer,
        Err(error) => {
            tracing::warn!(%error, "renewal windows not refreshed");
            return;
        }
    };
    for cert in due {
        match issuer.renewal_window(&cert.pem).await {
            Ok(at) => {
                if let Err(error) = set_renew_after(state, &cert.domain, Some(at)).await {
                    tracing::warn!(domain = %cert.domain, %error, "renewal window not stored");
                }
            }
            Err(error) => {
                tracing::info!(domain = %cert.domain, %error, "no renewal window from the CA")
            }
        }
    }
}

/// Every certificate on disk inside the CA's renewal window, or with a third of its life left
/// when the CA gave none, the panel's included.
pub async fn renew_due(
    state: &State,
    platform: &dyn Platform,
    issuance: &Issuance,
) -> anyhow::Result<Vec<String>> {
    let mut certs = Vec::new();
    for dir in platform.list_dir(Path::new(CERTS_DIR))? {
        let domain = acme::host_of_dir(&dir);
        let Some(pem) = platform.read_file(&acme::cert_dir(&domain).join("fullchain.pem"))? else {
            continue;
        };
        let Ok((not_before, not_after)) = acme::validity_of(&pem) else {
            continue;
        };
        certs.push(OnDisk {
            domain,
            pem,
            not_before,
            not_after,
        });
    }
    refresh_windows(state, issuance, &certs).await;

    let now = OffsetDateTime::now_utc();
    let mut renewed = Vec::new();
    for cert in certs {
        let window = renew_after(state, &cert.domain).await?;
        if acme::renew_due(cert.not_after, cert.not_before, now, window)
            && issue_one(state, platform, issuance, &cert.domain, true).await?
        {
            renewed.push(cert.domain);
        }
    }
    if !renewed.is_empty() {
        platform.nginx_test()?;
        platform.service(ServiceAction::Reload, NGINX_UNIT)?;
    }
    Ok(renewed)
}

pub async fn sweep(
    state: &State,
    platform: &dyn Platform,
    issuance: &Issuance,
) -> anyhow::Result<()> {
    for app in apps::list(state).await? {
        if let Err(e) = issue_for(state, platform, issuance, &app).await {
            tracing::warn!(app = %app.slug, error = ?e, "certificate sweep failed for an app");
        }
    }
    renew_due(state, platform, issuance).await?;
    Ok(())
}

pub fn spawn_sweeper(
    state: State,
    platform: Arc<dyn Platform>,
    issuance: Issuance,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tokio::time::sleep(FIRST_SWEEP).await;
        loop {
            if let Err(e) = sweep(&state, platform.as_ref(), &issuance).await {
                tracing::warn!(error = ?e, "certificate sweep failed");
            }
            tokio::time::sleep(SWEEP_INTERVAL).await;
        }
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use ferrum_platform::FakePlatform;

    const HERE: &str = "203.0.113.9";

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn unreachable_directory() -> Directory {
        Directory::Custom {
            url: "http://127.0.0.1:1/dir".into(),
            root_pem: None,
        }
    }

    async fn app_with_domain(state: &State) -> App {
        setup::set_email(state, "me@example.com").await.unwrap();
        apps::create(state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn issuance_is_not_attempted_while_dns_points_elsewhere() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = app_with_domain(&state).await;
        let issuance = Issuance::new(
            unreachable_directory(),
            Lookup::Fixed(vec![(
                "ledger.example.com".into(),
                vec![ip("198.51.100.1")],
            )]),
            Some(ip(HERE)),
        );
        assert!(!issue_for(&state, &p, &issuance, &app).await.unwrap());
        match status(&state, &p, "ledger.example.com").await.unwrap() {
            CertStatus::WaitingForDns { detail } => {
                assert!(detail.contains("198.51.100.1"), "{detail}");
                assert!(detail.contains(HERE), "{detail}");
            }
            other => panic!("{other:?}"),
        }
        let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM cert_attempts")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(attempts, 0, "a DNS wait is not an attempt against the CA");
        assert!(
            !p.calls()
                .iter()
                .any(|c| c.starts_with("write_file /etc/nginx"))
        );
    }

    #[tokio::test]
    async fn a_wildcard_goes_to_the_ca_over_dns_without_an_address_check() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        setup::set_email(&state, "me@example.com").await.unwrap();
        let sealed = crate::secrets::encrypt(&state.key, r#"{"token":"t"}"#);
        sqlx::query("INSERT INTO dns_providers (id, name, kind, credentials) VALUES ('cf', 'Cloudflare', 'cloudflare', ?)")
            .bind(&sealed)
            .execute(&state.pool)
            .await
            .unwrap();
        let mut new = new_app("ledger", &[("/", "main", false)]);
        let mut wildcard = apps::domains::NewDomain::from("*.ledger.example.com");
        wildcard.dns_provider_id = Some("cf".into());
        new.domains.push(wildcard);
        let app = apps::create(&state, new).await.unwrap();
        let issuance = Issuance::new(
            unreachable_directory(),
            Lookup::Fixed(vec![(
                "ledger.example.com".into(),
                vec![ip("198.51.100.1")],
            )]),
            Some(ip(HERE)),
        );
        assert!(!issuance.checks_dns());
        issue_for(&state, &p, &issuance, &app).await.unwrap();
        match status(&state, &p, "*.ledger.example.com").await.unwrap() {
            CertStatus::Failed { detail, .. } => assert!(detail.starts_with("acme:"), "{detail}"),
            other => panic!("{other:?}"),
        }
        let tried: Vec<(String, i64)> =
            sqlx::query_as("SELECT domain, attempts FROM cert_attempts ORDER BY domain")
                .fetch_all(&state.pool)
                .await
                .unwrap();
        assert_eq!(
            tried,
            [
                ("*.ledger.example.com".to_string(), 1),
                ("ledger.example.com".to_string(), 0)
            ],
            "the plain name waits for its A record, the wildcard does not"
        );
    }

    #[tokio::test]
    async fn a_failed_order_backs_off_and_records_why() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = app_with_domain(&state).await;
        let issuance = Issuance::new(
            unreachable_directory(),
            Lookup::Fixed(vec![("ledger.example.com".into(), vec![ip(HERE)])]),
            Some(ip(HERE)),
        );
        assert!(!issue_for(&state, &p, &issuance, &app).await.unwrap());
        let (attempts, waiting): (i64, bool) = sqlx::query_as(
            "SELECT attempts, next_at > datetime('now') FROM cert_attempts WHERE domain = 'ledger.example.com'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert_eq!(attempts, 1);
        assert!(waiting);
        match status(&state, &p, "ledger.example.com").await.unwrap() {
            CertStatus::Failed { detail, retry_at } => {
                assert!(
                    detail.starts_with("acme:"),
                    "the failure must come from the directory, not a second DNS lookup: {detail}"
                );
                assert!(retry_at.ends_with('Z'), "{retry_at}");
            }
            other => panic!("{other:?}"),
        }
        assert!(!issue_for(&state, &p, &issuance, &app).await.unwrap());
        let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM cert_attempts")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(attempts, 1, "a second call inside the backoff does nothing");
        retry_now(&state, &app).await.unwrap();
        assert_eq!(
            status(&state, &p, "ledger.example.com").await.unwrap(),
            CertStatus::None
        );
    }

    #[tokio::test]
    async fn the_sweep_gives_a_site_its_tls_blocks_once_its_certificate_is_on_disk() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = app_with_domain(&state).await;
        provision::provision(&state, &p, &app).await.unwrap();
        let site = "/etc/nginx/conf.d/ferrum-ledger.conf";
        assert!(!p.written(site).unwrap().contains("listen 443"));
        p.write_file(
            &acme::cert_dir("ledger.example.com").join("fullchain.pem"),
            &self_signed("ledger.example.com", 60),
            0o644,
        )
        .unwrap();
        let issuance = Issuance::new(
            unreachable_directory(),
            Lookup::Fixed(vec![]),
            Some(ip(HERE)),
        );
        let writes = |p: &FakePlatform| {
            p.calls()
                .iter()
                .filter(|c| c.starts_with(&format!("write_file {site}")))
                .count()
        };
        let before = writes(&p);

        assert!(issue_for(&state, &p, &issuance, &app).await.unwrap());
        assert!(p.written(site).unwrap().contains("listen 443 ssl;"));
        assert_eq!(writes(&p), before + 1);
        assert_eq!(p.calls_matching("service reload nginx").len(), 2);

        assert!(!issue_for(&state, &p, &issuance, &app).await.unwrap());
        assert_eq!(writes(&p), before + 1, "a matching site is left alone");
    }

    /// A ninety-day certificate with `days_left` to run.
    pub(crate) fn self_signed(domain: &str, days_left: i64) -> String {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec![domain.to_string()]).unwrap();
        let now = time::OffsetDateTime::now_utc();
        params.not_after = now + time::Duration::days(days_left);
        params.not_before = params.not_after - time::Duration::days(90);
        params.self_signed(&key).unwrap().pem()
    }

    #[tokio::test]
    async fn the_ca_window_decides_renewal_when_there_is_one() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        setup::set_email(&state, "me@example.com").await.unwrap();
        for (domain, days) in [("early.example.com", 60), ("late.example.com", 10)] {
            p.write_file(
                &acme::cert_dir(domain).join("fullchain.pem"),
                &self_signed(domain, days),
                0o644,
            )
            .unwrap();
        }
        let now = OffsetDateTime::now_utc();
        set_renew_after(
            &state,
            "early.example.com",
            Some(now - time::Duration::hours(1)),
        )
        .await
        .unwrap();
        set_renew_after(
            &state,
            "late.example.com",
            Some(now + time::Duration::days(2)),
        )
        .await
        .unwrap();
        let issuance = Issuance::new(
            unreachable_directory(),
            Lookup::Fixed(vec![
                ("early.example.com".into(), vec![ip(HERE)]),
                ("late.example.com".into(), vec![ip(HERE)]),
            ]),
            Some(ip(HERE)),
        );
        renew_due(&state, &p, &issuance).await.unwrap();
        let tried: Vec<(String, i64)> =
            sqlx::query_as("SELECT domain, attempts FROM cert_attempts ORDER BY domain")
                .fetch_all(&state.pool)
                .await
                .unwrap();
        assert_eq!(
            tried,
            vec![
                ("early.example.com".to_string(), 1),
                ("late.example.com".to_string(), 0)
            ]
        );
        assert!(
            renew_after(&state, "early.example.com")
                .await
                .unwrap()
                .is_some(),
            "a failed renewal keeps the window"
        );
    }

    #[tokio::test]
    async fn a_renewal_out_of_quick_retries_tells_the_owner_once() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = app_with_domain(&state).await;
        p.write_file(
            &acme::cert_dir("ledger.example.com").join("fullchain.pem"),
            &self_signed("ledger.example.com", 10),
            0o644,
        )
        .unwrap();
        sqlx::query("INSERT INTO cert_attempts (domain, attempts, next_at) VALUES ('ledger.example.com', 4, datetime('now', '-1 minute'))")
            .execute(&state.pool)
            .await
            .unwrap();
        let issuance = Issuance::new(
            unreachable_directory(),
            Lookup::Fixed(vec![("ledger.example.com".into(), vec![ip(HERE)])]),
            Some(ip(HERE)),
        );
        renew_due(&state, &p, &issuance).await.unwrap();
        renew_due(&state, &p, &issuance).await.unwrap();

        let events = events::list(&state, 10, false).await.unwrap();
        assert_eq!(events.len(), 1, "{events:?}");
        let e = &events[0];
        assert_eq!(e.kind, "broke_on_its_own");
        assert_eq!(e.app_id.as_deref(), Some(app.id.as_str()));
        assert_eq!(e.subject, "ledger.example.com");
        assert_eq!(e.link.as_deref(), Some("/apps/ledger"));
        assert!(
            e.sentence
                .starts_with("The certificate for ledger.example.com could not be renewed: acme:"),
            "{}",
            e.sentence
        );
    }

    #[tokio::test]
    async fn a_certificate_on_disk_reports_its_expiry_and_only_a_short_one_is_renewed() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = app_with_domain(&state).await;
        p.write_file(
            &acme::cert_dir("ledger.example.com").join("fullchain.pem"),
            &self_signed("ledger.example.com", 60),
            0o644,
        )
        .unwrap();
        p.write_file(
            &acme::cert_dir("old.example.com").join("fullchain.pem"),
            &self_signed("old.example.com", 10),
            0o644,
        )
        .unwrap();
        let issuance = Issuance::new(
            unreachable_directory(),
            Lookup::Fixed(vec![("old.example.com".into(), vec![ip(HERE)])]),
            Some(ip(HERE)),
        );
        assert!(!issue_for(&state, &p, &issuance, &app).await.unwrap());
        match status(&state, &p, "ledger.example.com").await.unwrap() {
            CertStatus::Issued { not_after } => assert!(not_after.ends_with('Z'), "{not_after}"),
            other => panic!("{other:?}"),
        }
        assert!(
            p.calls()
                .iter()
                .all(|c| !c.starts_with("write_file /etc/nginx"))
        );

        assert_eq!(
            renew_due(&state, &p, &issuance).await.unwrap(),
            Vec::<String>::new()
        );
        let attempts: Vec<(String, i64)> =
            sqlx::query_as("SELECT domain, attempts FROM cert_attempts")
                .fetch_all(&state.pool)
                .await
                .unwrap();
        assert_eq!(
            attempts,
            vec![("old.example.com".to_string(), 1)],
            "only the certificate past two thirds of its life was tried"
        );
        assert!(!p.calls().iter().any(|c| c == "service reload nginx"));
    }

    #[test]
    fn backoff_doubles_then_gives_up_for_a_day() {
        assert_eq!(backoff_secs(1), 60);
        assert_eq!(backoff_secs(2), 120);
        assert_eq!(backoff_secs(4), 480);
        assert_eq!(backoff_secs(5), GIVE_UP_SECS);
        assert_eq!(backoff_secs(9), GIVE_UP_SECS);
    }
}
