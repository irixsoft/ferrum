use crate::dns::{self, Verdict};
use crate::state::State;
use crate::{ACME_WEBROOT, CERTS_DIR};
use instant_acme::{
    Account, AccountCredentials, ChallengeType, Identifier, LetsEncrypt, NewAccount, NewOrder,
    OrderStatus, RetryPolicy,
};
use std::net::IpAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use time::OffsetDateTime;

const ACCOUNT_SETTING: &str = "acme.account";
const RENEW_AT_DAYS: i64 = 30;

#[derive(Debug, thiserror::Error)]
pub enum AcmeError {
    #[error("{0}")]
    DnsNotReady(String),
    #[error("dns: {0}")]
    Dns(#[from] dns::DnsLookupError),
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

impl Directory {
    pub fn url(&self) -> &str {
        match self {
            Self::LetsEncrypt => LetsEncrypt::Production.url(),
            Self::Staging => LetsEncrypt::Staging.url(),
            Self::Custom { url, .. } => url,
        }
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
}

pub fn cert_dir(host: &str) -> PathBuf {
    Path::new(CERTS_DIR).join(host)
}

pub fn challenge_path(token: &str) -> PathBuf {
    Path::new(ACME_WEBROOT).join(token)
}

pub fn renew_due(not_after: OffsetDateTime, now: OffsetDateTime) -> bool {
    not_after - now < time::Duration::days(RENEW_AT_DAYS)
}

pub fn not_after_of(pem: &str) -> Result<OffsetDateTime, AcmeError> {
    let (_, der) = x509_parser::pem::parse_x509_pem(pem.as_bytes())
        .map_err(|e| AcmeError::Expiry(e.to_string()))?;
    let cert = der
        .parse_x509()
        .map_err(|e| AcmeError::Expiry(e.to_string()))?;
    Ok(cert.validity().not_after.to_datetime())
}

#[derive(Debug, Clone)]
pub struct Backoff {
    attempt: u32,
    max_attempts: u32,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            attempt: 0,
            max_attempts: 5,
        }
    }
}

impl Backoff {
    pub fn next_delay(&mut self) -> Option<Duration> {
        if self.attempt >= self.max_attempts {
            return None;
        }
        let secs = 30u64 << self.attempt;
        self.attempt += 1;
        Some(Duration::from_secs(secs))
    }
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

    pub async fn issue(
        &self,
        host: &str,
        expected: IpAddr,
        dir: &Path,
    ) -> Result<Certificate, AcmeError> {
        let verdict = dns::verify(host, expected).await?;
        if verdict != Verdict::Match {
            return Err(AcmeError::DnsNotReady(dns::describe(&verdict, host)));
        }

        let identifiers = [Identifier::Dns(host.to_string())];
        let mut order = self.account.new_order(&NewOrder::new(&identifiers)).await?;

        let written = self.prepare_challenges(&mut order, host).await?;
        let result = self.complete(&mut order, host, dir).await;
        for path in written {
            let _ = std::fs::remove_file(path);
        }
        result
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
    fn renewal_is_due_at_thirty_days_remaining() {
        let now = OffsetDateTime::now_utc();
        assert!(!renew_due(now + TimeDuration::days(45), now));
        assert!(renew_due(now + TimeDuration::days(29), now));
        assert!(renew_due(now - TimeDuration::days(1), now));
    }

    #[test]
    fn backoff_grows_and_then_gives_up() {
        let mut b = Backoff::default();
        let first = b.next_delay().unwrap();
        let second = b.next_delay().unwrap();
        assert!(second > first);
        for _ in 0..10 {
            b.next_delay();
        }
        assert!(
            b.next_delay().is_none(),
            "backoff must terminate, never loop"
        );
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
