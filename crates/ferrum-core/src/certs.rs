use crate::acme::{self, Directory, Issuer};
use crate::apps::{self, App, provision};
use crate::dns::{self, Lookup, Verdict};
use crate::state::State;
use crate::{CERTS_DIR, setup};
use anyhow::Context;
use ferrum_platform::ubuntu::NGINX_UNIT;
use ferrum_platform::{Platform, ServiceAction};
use serde::Serialize;
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const SWEEP_INTERVAL: Duration = Duration::from_secs(5 * 60);
pub const FIRST_SWEEP: Duration = Duration::from_secs(30);
const DNS_RETRY_SECS: i64 = 5 * 60;
const MAX_ATTEMPTS: i64 = 5;
const GIVE_UP_SECS: i64 = 24 * 60 * 60;
const BASE_BACKOFF_SECS: i64 = 30;

#[derive(Clone)]
pub struct Issuance {
    pub directory: Directory,
    pub resolver: Lookup,
    public_ip: Arc<Mutex<Option<IpAddr>>>,
}

impl Issuance {
    pub fn new(directory: Directory, resolver: Lookup, public_ip: Option<IpAddr>) -> Self {
        Self {
            directory,
            resolver,
            public_ip: Arc::new(Mutex::new(public_ip)),
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

async fn clear(state: &State, domain: &str) -> anyhow::Result<()> {
    sqlx::query!("DELETE FROM cert_attempts WHERE domain = ?", domain)
        .execute(&state.pool)
        .await?;
    Ok(())
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
        Some(a) if a.attempts == 0 => CertStatus::WaitingForDns {
            detail: a.last_error.unwrap_or_default(),
        },
        Some(a) => CertStatus::Failed {
            detail: a.last_error.unwrap_or_default(),
            retry_at: crate::time::utc(a.next_at.unwrap_or_default()),
        },
        None => CertStatus::None,
    })
}

pub async fn statuses(
    state: &State,
    platform: &dyn Platform,
    app: &App,
) -> anyhow::Result<Vec<DomainCert>> {
    let mut out = Vec::with_capacity(app.domains.len());
    for domain in &app.domains {
        out.push(DomainCert {
            domain: domain.clone(),
            status: status(state, platform, domain).await?,
        });
    }
    Ok(out)
}

/// Clears the backoff so the next sweep tries at once.
pub async fn retry_now(state: &State, app: &App) -> anyhow::Result<()> {
    for domain in &app.domains {
        clear(state, domain).await?;
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
        Ok(issuer) => {
            issuer
                .issue(domain, expected, &acme::cert_dir(domain))
                .await
        }
        Err(e) => Err(e),
    };
    match issued {
        Ok(_) => {
            clear(state, domain).await?;
            tracing::info!(domain, "certificate issued");
            Ok(true)
        }
        Err(e) => {
            let attempts = attempts + 1;
            tracing::warn!(domain, attempts, error = %e, "certificate issuance failed");
            record(
                state,
                domain,
                attempts,
                &e.to_string(),
                backoff_secs(attempts),
            )
            .await?;
            Ok(false)
        }
    }
}

/// Issues for every domain of the app that has no certificate; `true` when one landed and the
/// vhost was re-rendered with it.
pub async fn issue_for(
    state: &State,
    platform: &dyn Platform,
    issuance: &Issuance,
    app: &App,
) -> anyhow::Result<bool> {
    let mut landed = false;
    for domain in &app.domains {
        landed |= try_issue(state, platform, issuance, domain, false).await?;
    }
    if landed {
        provision::provision(state, platform, app).await?;
    }
    Ok(landed)
}

/// Every certificate on disk with under thirty days left, the panel's included.
pub async fn renew_due(
    state: &State,
    platform: &dyn Platform,
    issuance: &Issuance,
) -> anyhow::Result<Vec<String>> {
    let now = time::OffsetDateTime::now_utc();
    let mut renewed = Vec::new();
    for domain in platform.list_dir(Path::new(CERTS_DIR))? {
        let Some(pem) = platform.read_file(&acme::cert_dir(&domain).join("fullchain.pem"))? else {
            continue;
        };
        let Ok(not_after) = acme::not_after_of(&pem) else {
            continue;
        };
        if acme::renew_due(not_after, now)
            && try_issue(state, platform, issuance, &domain, true).await?
        {
            renewed.push(domain);
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
mod tests {
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
                assert!(!detail.is_empty());
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

    fn self_signed(domain: &str, days_left: i64) -> String {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec![domain.to_string()]).unwrap();
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(days_left);
        params.self_signed(&key).unwrap().pem()
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
            "only the certificate under thirty days was tried"
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
