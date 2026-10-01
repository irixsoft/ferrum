use crate::dns_providers::{DnsProvider, TxtHandle};
use crate::state::State;
use crate::{ACME_WEBROOT, CERTS_DIR};
use instant_acme::{
    Account, AccountCredentials, CertificateIdentifier, ChallengeType, Identifier, LetsEncrypt,
    NewAccount, NewOrder, OrderStatus, RetryPolicy,
};
use rustls::pki_types::CertificateDer;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use time::OffsetDateTime;

const ACCOUNT_SETTING: &str = "acme.account";
const DIRECTORY_SETTING: &str = "acme.directory";
const WILDCARD_DIR_PREFIX: &str = "_wildcard.";
const TXT_DEADLINE: Duration = Duration::from_secs(120);

#[derive(Debug, thiserror::Error)]
pub enum AcmeError {
    #[error("acme: {0}")]
    Acme(#[from] instant_acme::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("state: {0}")]
    State(String),
    #[error("the certificate authority did not return a certificate for {0}")]
    NoCertificate(String),
    #[error("could not read the expiry of the issued certificate: {0}")]
    Expiry(String),
    #[error("{0}")]
    Dns(String),
}

#[derive(Debug, Clone)]
pub enum Directory {
    LetsEncrypt,
    Staging,
    Custom {
        url: String,
        root_pem: Option<PathBuf>,
    },
}

pub async fn set_directory(state: &State, staging: bool) -> anyhow::Result<()> {
    let name = if staging { "staging" } else { "production" };
    state.set_setting(DIRECTORY_SETTING, name).await
}

/// What `ferrum setup` chose; a box set up with `--staging` keeps it until a production setup.
pub async fn directory(state: &State) -> anyhow::Result<Directory> {
    Ok(
        match state.get_setting(DIRECTORY_SETTING).await?.as_deref() {
            Some("staging") => Directory::Staging,
            _ => Directory::LetsEncrypt,
        },
    )
}

impl Directory {
    pub fn url(&self) -> &str {
        match self {
            Self::LetsEncrypt => LetsEncrypt::Production.url(),
            Self::Staging => LetsEncrypt::Staging.url(),
            Self::Custom { url, .. } => url,
        }
    }

    /// A test CA such as Pebble answers from its own resolver, so public propagation means nothing.
    pub fn is_custom(&self) -> bool {
        matches!(self, Self::Custom { .. })
    }

    fn builder(&self) -> Result<instant_acme::AccountBuilder, AcmeError> {
        Ok(match self {
            Self::Custom {
                root_pem: Some(pem),
                ..
            } => Account::builder_with_root(pem)?,
            _ => Account::builder()?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Certificate {
    pub fullchain: PathBuf,
    pub key: PathBuf,
    pub not_after: OffsetDateTime,
    pub renew_after: Option<OffsetDateTime>,
}

/// A wildcard's directory is spelled `_wildcard.<base>`; no hostname can start with `_`.
pub fn cert_dir(host: &str) -> PathBuf {
    match host.strip_prefix("*.") {
        Some(base) => Path::new(CERTS_DIR).join(format!("{WILDCARD_DIR_PREFIX}{base}")),
        None => Path::new(CERTS_DIR).join(host),
    }
}

pub fn host_of_dir(dir_name: &str) -> String {
    match dir_name.strip_prefix(WILDCARD_DIR_PREFIX) {
        Some(base) => format!("*.{base}"),
        None => dir_name.to_string(),
    }
}

pub fn challenge_path(token: &str) -> PathBuf {
    Path::new(ACME_WEBROOT).join(token)
}

/// The CA's suggested instant when it gave one, else once a third of the lifetime remains.
pub fn renew_due(
    not_after: OffsetDateTime,
    not_before: OffsetDateTime,
    now: OffsetDateTime,
    renew_after: Option<OffsetDateTime>,
) -> bool {
    match renew_after {
        Some(at) => now >= at,
        None => (not_after - now) * 3 < not_after - not_before,
    }
}

fn parse_pem(pem: &str) -> Result<x509_parser::pem::Pem, AcmeError> {
    let (_, der) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|e| AcmeError::Expiry(e.to_string()))?;
    Ok(der)
}

/// `(not_before, not_after)` of the first certificate in `pem`.
pub fn validity_of(pem: &str) -> Result<(OffsetDateTime, OffsetDateTime), AcmeError> {
    let der = parse_pem(pem)?;
    let cert = der
        .parse_x509()
        .map_err(|e| AcmeError::Expiry(e.to_string()))?;
    let validity = cert.validity();
    Ok((
        validity.not_before.to_datetime(),
        validity.not_after.to_datetime(),
    ))
}

pub fn not_after_of(pem: &str) -> Result<OffsetDateTime, AcmeError> {
    Ok(validity_of(pem)?.1)
}

/// The ARI identifier of the first certificate in `pem`; a self-signed one has none.
pub fn cert_id_of(pem: &str) -> Result<CertificateIdentifier<'static>, AcmeError> {
    let der = CertificateDer::from(parse_pem(pem)?.contents);
    CertificateIdentifier::try_from(&der)
        .map(CertificateIdentifier::into_owned)
        .map_err(AcmeError::Expiry)
}

pub fn instant_in(start: OffsetDateTime, end: OffsetDateTime) -> OffsetDateTime {
    let span = (end - start).whole_seconds();
    if span <= 0 {
        return start;
    }
    start + time::Duration::seconds(rand::random_range(0..=span))
}

pub struct Issuer {
    account: Account,
    webroot: PathBuf,
}

impl Issuer {
    pub async fn new(
        state: &State,
        directory: Directory,
        contact_email: &str,
    ) -> Result<Self, AcmeError> {
        let stored = state
            .get_setting(ACCOUNT_SETTING)
            .await
            .map_err(|e| AcmeError::State(e.to_string()))?;

        let account = match stored {
            Some(json) => {
                let creds: AccountCredentials = serde_json::from_str(&json)
                    .map_err(|e| AcmeError::State(format!("stored ACME account: {e}")))?;
                directory.builder()?.from_credentials(creds).await?
            }
            None => {
                let contact = format!("mailto:{contact_email}");
                let (account, creds) = directory
                    .builder()?
                    .create(
                        &NewAccount {
                            contact: &[&contact],
                            terms_of_service_agreed: true,
                            only_return_existing: false,
                        },
                        directory.url().to_string(),
                        None,
                    )
                    .await?;
                let json = serde_json::to_string(&creds)
                    .map_err(|e| AcmeError::State(format!("serialising ACME account: {e}")))?;
                state
                    .set_setting(ACCOUNT_SETTING, &json)
                    .await
                    .map_err(|e| AcmeError::State(e.to_string()))?;
                account
            }
        };

        Ok(Self {
            account,
            webroot: PathBuf::from(ACME_WEBROOT),
        })
    }

    pub fn with_webroot(mut self, webroot: PathBuf) -> Self {
        self.webroot = webroot;
        self
    }

    /// The caller must confirm the host resolves here first; this goes straight to the directory.
    pub async fn issue(&self, host: &str, dir: &Path) -> Result<Certificate, AcmeError> {
        let identifiers = [Identifier::Dns(host.to_string())];
        let mut order = self.new_order(&identifiers, dir).await?;

        let written = self.prepare_challenges(&mut order, host).await?;
        let result = self.complete(&mut order, host, dir).await;
        for path in written {
            let _ = std::fs::remove_file(path);
        }
        result
    }

    /// DNS-01 through `provider`; `host` may be `*.example.com`. With `check`, each record must
    /// reach every authoritative nameserver before the CA is told to look.
    pub async fn issue_dns(
        &self,
        host: &str,
        dir: &Path,
        provider: &dyn DnsProvider,
        check: bool,
    ) -> Result<Certificate, AcmeError> {
        let identifiers = [Identifier::Dns(host.to_string())];
        let mut order = self.new_order(&identifiers, dir).await?;

        let mut records = Vec::new();
        let result = match self
            .prepare_dns(&mut order, host, provider, check, &mut records)
            .await
        {
            Ok(()) => self.complete(&mut order, host, dir).await,
            Err(e) => Err(e),
        };
        for record in records {
            let fqdn = record.fqdn.clone();
            if let Err(error) = provider.delete_txt(record).await {
                tracing::warn!(%fqdn, error = %format!("{error:#}"), "challenge record not removed");
            }
        }
        result
    }

    /// Marks the order as the successor of the certificate already in `dir`, where the CA allows.
    async fn new_order(
        &self,
        identifiers: &[Identifier],
        dir: &Path,
    ) -> Result<instant_acme::Order, AcmeError> {
        let previous = std::fs::read_to_string(dir.join("fullchain.pem"))
            .ok()
            .and_then(|pem| cert_id_of(&pem).ok());
        if let Some(id) = previous {
            match self
                .account
                .new_order(&NewOrder::new(identifiers).replaces(id))
                .await
            {
                Ok(order) => return Ok(order),
                Err(error) => {
                    tracing::info!(%error, "the CA refused the order as a replacement; ordering afresh")
                }
            }
        }
        Ok(self.account.new_order(&NewOrder::new(identifiers)).await?)
    }

    async fn prepare_dns(
        &self,
        order: &mut instant_acme::Order,
        host: &str,
        provider: &dyn DnsProvider,
        check: bool,
        records: &mut Vec<TxtHandle>,
    ) -> Result<(), AcmeError> {
        let mut authorizations = order.authorizations();
        while let Some(handle) = authorizations.next().await {
            let mut handle = handle?;
            if handle.status == instant_acme::AuthorizationStatus::Valid {
                continue;
            }
            let mut challenge = handle
                .challenge(ChallengeType::Dns01)
                .ok_or_else(|| AcmeError::NoCertificate(host.to_string()))?;
            let base = match challenge.identifier().identifier {
                Identifier::Dns(name) => name.trim_start_matches("*.").to_string(),
                _ => return Err(AcmeError::NoCertificate(host.to_string())),
            };
            let fqdn = format!("_acme-challenge.{base}");
            let value = challenge.key_authorization().dns_value();
            let record = provider
                .create_txt(&fqdn, &value)
                .await
                .map_err(|e| AcmeError::Dns(format!("{e:#}")))?;
            records.push(record);
            if check {
                crate::dns::txt_visible_on_authoritatives(&fqdn, &value, TXT_DEADLINE)
                    .await
                    .map_err(|e| AcmeError::Dns(format!("{e:#}")))?;
            }
            challenge.set_ready().await?;
        }
        Ok(())
    }

    /// A random instant inside the window the CA suggests for replacing the certificate in `pem`.
    pub async fn renewal_window(&self, pem: &str) -> Result<OffsetDateTime, AcmeError> {
        let id = cert_id_of(pem)?;
        let (info, _) = self.account.renewal_info(&id).await?;
        Ok(instant_in(
            info.suggested_window.start,
            info.suggested_window.end,
        ))
    }

    async fn prepare_challenges(
        &self,
        order: &mut instant_acme::Order,
        host: &str,
    ) -> Result<Vec<PathBuf>, AcmeError> {
        let mut written = Vec::new();
        let mut authorizations = order.authorizations();
        while let Some(handle) = authorizations.next().await {
            let mut handle = handle?;
            if handle.status == instant_acme::AuthorizationStatus::Valid {
                continue;
            }
            let mut challenge = handle
                .challenge(ChallengeType::Http01)
                .ok_or_else(|| AcmeError::NoCertificate(host.to_string()))?;
            let token = challenge.token.clone();
            let authorization = challenge.key_authorization();

            std::fs::create_dir_all(&self.webroot)?;
            let path = self.webroot.join(&token);
            std::fs::write(&path, authorization.as_str())?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
            written.push(path);

            challenge.set_ready().await?;
        }
        Ok(written)
    }

    async fn complete(
        &self,
        order: &mut instant_acme::Order,
        host: &str,
        dir: &Path,
    ) -> Result<Certificate, AcmeError> {
        let policy = RetryPolicy::default().timeout(Duration::from_secs(120));
        let status = order.poll_ready(&policy).await?;
        if status != OrderStatus::Ready {
            return Err(AcmeError::NoCertificate(host.to_string()));
        }

        let key_pem = order.finalize().await?;
        let chain_pem = order.poll_certificate(&policy).await?;
        let not_after = not_after_of(&chain_pem)?;
        let renew_after = match self.renewal_window(&chain_pem).await {
            Ok(at) => Some(at),
            Err(error) => {
                tracing::info!(host, %error, "no renewal window from the CA");
                None
            }
        };

        std::fs::create_dir_all(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755))?;

        let fullchain = dir.join("fullchain.pem");
        let key = dir.join("key.pem");
        write_secret(&fullchain, &chain_pem, 0o644)?;
        write_secret(&key, &key_pem, 0o600)?;

        Ok(Certificate {
            fullchain,
            key,
            not_after,
            renew_after,
        })
    }
}

fn write_secret(path: &Path, contents: &str, mode: u32) -> Result<(), AcmeError> {
    std::fs::write(path, contents)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::{Duration as TimeDuration, OffsetDateTime};

    #[test]
    fn renewal_follows_the_ca_window_else_a_third_of_the_lifetime() {
        let now = OffsetDateTime::now_utc();
        let days = TimeDuration::days;
        let issued = now - days(50);
        assert!(!renew_due(issued + days(90), issued, now, None));
        assert!(renew_due(issued + days(70), issued, now, None));
        assert!(renew_due(now - days(1), now - days(91), now, None));

        let short = now - days(3);
        assert!(!renew_due(short + days(6), short, now, None));
        assert!(renew_due(short + days(4), short, now, None));

        assert!(renew_due(
            issued + days(90),
            issued,
            now,
            Some(now - days(1))
        ));
        assert!(!renew_due(
            issued + days(10),
            issued,
            now,
            Some(now + days(1))
        ));
    }

    #[test]
    fn an_instant_in_the_window_stays_inside_it() {
        let start = OffsetDateTime::now_utc();
        let end = start + TimeDuration::hours(6);
        for _ in 0..50 {
            let at = instant_in(start, end);
            assert!(at >= start && at <= end);
        }
        assert_eq!(instant_in(end, start), end);
    }

    #[test]
    fn a_wildcard_lives_in_a_directory_no_hostname_can_take() {
        let dir = cert_dir("*.example.com");
        assert_eq!(
            dir.to_string_lossy(),
            "/var/lib/ferrum/certs/_wildcard.example.com"
        );
        assert_eq!(host_of_dir("_wildcard.example.com"), "*.example.com");
        assert_eq!(host_of_dir("panel.example.com"), "panel.example.com");
    }

    #[test]
    fn a_self_signed_certificate_has_no_renewal_identifier() {
        let pem = crate::certs::tests::self_signed("example.com", 60);
        assert!(cert_id_of(&pem).is_err());
        let (before, after) = validity_of(&pem).unwrap();
        assert!(after - before > TimeDuration::days(89));
    }

    #[test]
    fn challenge_path_matches_the_nginx_root() {
        let p = challenge_path("TOKEN123");
        assert_eq!(
            p.to_string_lossy(),
            "/var/lib/ferrum/acme/.well-known/acme-challenge/TOKEN123"
        );
    }

    #[test]
    fn cert_dir_is_per_host() {
        assert_eq!(
            cert_dir("panel.example.com").to_string_lossy(),
            "/var/lib/ferrum/certs/panel.example.com"
        );
    }

    #[test]
    fn staging_and_production_directories_differ() {
        assert_ne!(Directory::LetsEncrypt.url(), Directory::Staging.url());
        assert!(Directory::Staging.url().contains("staging"));
    }
}
