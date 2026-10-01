mod cloudflare;
pub mod fake;
mod route53;
mod sigv4;

pub use cloudflare::Cloudflare;
pub use route53::Route53;

use crate::dns;
use crate::secrets;
use crate::state::State;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

const CLOUDFLARE_API: &str = "https://api.cloudflare.com/client/v4";
const ROUTE53_API: &str = "https://route53.amazonaws.com";
const CLOUDFLARE_API_SETTING: &str = "dns.cloudflare_api";
const ROUTE53_API_SETTING: &str = "dns.route53_api";

pub type Pending<'a, T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxtHandle {
    pub zone: String,
    pub record: String,
    pub fqdn: String,
    pub value: String,
}

pub trait DnsProvider: Send + Sync {
    fn create_txt<'a>(&'a self, fqdn: &'a str, value: &'a str) -> Pending<'a, TxtHandle>;
    fn delete_txt<'a>(&'a self, handle: TxtHandle) -> Pending<'a, ()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Cloudflare,
    Route53,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Cloudflare => "cloudflare",
            Kind::Route53 => "route53",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        [Kind::Cloudflare, Kind::Route53]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewProvider {
    pub name: String,
    pub kind: Kind,
    pub credentials: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    InUse(String),
    #[error("No DNS provider has that id")]
    NotFound,
}

enum Credentials {
    Cloudflare {
        token: String,
    },
    Route53 {
        access_key_id: String,
        secret_access_key: String,
    },
}

fn field(value: &Value, key: &str, label: &str) -> Result<String, ProviderError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ProviderError::Invalid(format!("Enter the {label}")))
}

impl Credentials {
    fn parse(kind: Kind, value: &Value) -> Result<Self, ProviderError> {
        Ok(match kind {
            Kind::Cloudflare => Self::Cloudflare {
                token: field(value, "token", "Cloudflare API token")?,
            },
            Kind::Route53 => Self::Route53 {
                access_key_id: field(value, "access_key_id", "AWS access key ID")?,
                secret_access_key: field(value, "secret_access_key", "AWS secret access key")?,
            },
        })
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Cloudflare { token } => serde_json::json!({ "token": token }),
            Self::Route53 {
                access_key_id,
                secret_access_key,
            } => serde_json::json!({
                "access_key_id": access_key_id,
                "secret_access_key": secret_access_key,
            }),
        }
    }

    async fn client(self, state: &State, http: &reqwest::Client) -> Box<dyn DnsProvider> {
        match self {
            Self::Cloudflare { token } => {
                let base = endpoint(state, CLOUDFLARE_API_SETTING, CLOUDFLARE_API).await;
                Box::new(Cloudflare::new(http.clone(), base, token))
            }
            Self::Route53 {
                access_key_id,
                secret_access_key,
            } => {
                let base = endpoint(state, ROUTE53_API_SETTING, ROUTE53_API).await;
                Box::new(Route53::new(
                    http.clone(),
                    base,
                    access_key_id,
                    secret_access_key,
                ))
            }
        }
    }
}

async fn endpoint(state: &State, setting: &str, default: &str) -> String {
    state
        .get_setting(setting)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| default.to_string())
}

/// Creates and removes a throwaway TXT record, so a token without DNS edit rights fails here.
pub async fn probe(client: &dyn DnsProvider, zone: &str) -> anyhow::Result<()> {
    let fqdn = format!("_ferrum-probe.{zone}");
    let value = crate::secret::generate();
    let handle = client.create_txt(&fqdn, &value).await?;
    client.delete_txt(handle).await
}

/// Validates the credentials, proves them against `zone`, then stores them sealed.
pub async fn create(
    state: &State,
    http: &reqwest::Client,
    new: NewProvider,
    zone: &str,
) -> anyhow::Result<Provider> {
    let name = new.name.trim().to_string();
    if name.is_empty() {
        return Err(ProviderError::Invalid("Give the provider a name".into()).into());
    }
    if name.chars().count() > 64 {
        return Err(ProviderError::Invalid("Keep the name under 64 characters".into()).into());
    }
    let zone = dns::validate_hostname(zone).map_err(ProviderError::Invalid)?;
    let credentials = Credentials::parse(new.kind, &new.credentials)?;
    if list(state).await?.iter().any(|p| p.name == name) {
        return Err(
            ProviderError::Invalid(format!("A provider named {name} already exists")).into(),
        );
    }
    let sealed = secrets::encrypt(&state.key, &credentials.to_json().to_string());
    let client = credentials.client(state, http).await;
    probe(client.as_ref(), &zone)
        .await
        .map_err(|e| ProviderError::Invalid(format!("{e:#}")))?;

    let id = uuid::Uuid::new_v4().to_string();
    let kind = new.kind.as_str();
    sqlx::query!(
        "INSERT INTO dns_providers (id, name, kind, credentials) VALUES (?, ?, ?, ?)",
        id,
        name,
        kind,
        sealed
    )
    .execute(&state.pool)
    .await?;
    get(state, &id)
        .await?
        .ok_or_else(|| ProviderError::NotFound.into())
}

pub async fn get(state: &State, id: &str) -> anyhow::Result<Option<Provider>> {
    Ok(list(state).await?.into_iter().find(|p| p.id == id))
}

pub async fn list(state: &State) -> anyhow::Result<Vec<Provider>> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", name, kind, created_at FROM dns_providers ORDER BY name"#
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(Provider {
                kind: Kind::parse(&r.kind)?,
                id: r.id,
                name: r.name,
                created_at: crate::time::utc(r.created_at),
            })
        })
        .collect())
}

pub async fn remove(state: &State, id: &str) -> anyhow::Result<()> {
    let provider = get(state, id).await?.ok_or(ProviderError::NotFound)?;
    let used = sqlx::query_scalar!(
        "SELECT domain FROM app_domains WHERE dns_provider_id = ? ORDER BY domain LIMIT 1",
        id
    )
    .fetch_optional(&state.pool)
    .await?;
    if let Some(domain) = used {
        return Err(ProviderError::InUse(format!(
            "{} issues the certificate for {domain}; give that domain another provider first",
            provider.name
        ))
        .into());
    }
    sqlx::query!("DELETE FROM dns_providers WHERE id = ?", id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

pub async fn client(
    state: &State,
    http: &reqwest::Client,
    id: &str,
) -> anyhow::Result<Box<dyn DnsProvider>> {
    let row = sqlx::query!(
        "SELECT kind, credentials FROM dns_providers WHERE id = ?",
        id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ProviderError::NotFound)?;
    let kind = Kind::parse(&row.kind).ok_or(ProviderError::NotFound)?;
    let plain = secrets::decrypt(&state.key, &row.credentials)?;
    let value: Value = serde_json::from_str(&plain)?;
    Ok(Credentials::parse(kind, &value)?.client(state, http).await)
}

/// A provider's error body, cut short so a page of HTML never becomes the sentence.
fn excerpt(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(200) {
        Some((i, _)) => format!("{}…", &flat[..i]),
        None => flat,
    }
}

#[cfg(test)]
pub(crate) mod stub {
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Debug, Clone)]
    pub struct Seen {
        pub method: String,
        pub path: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    impl Seen {
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    pub struct Stub {
        pub base: String,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl Stub {
        pub async fn serve<F>(reply: F) -> Self
        where
            F: Fn(&Seen) -> (u16, String) + Send + Sync + 'static,
        {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let log = seen.clone();
            tokio::spawn(async move {
                while let Ok((mut socket, _)) = listener.accept().await {
                    let Some(req) = read(&mut socket).await else {
                        continue;
                    };
                    let (status, body) = reply(&req);
                    log.lock().unwrap().push(req);
                    let head = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                }
            });
            Self { base, seen }
        }

        pub fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
    }

    async fn read(socket: &mut tokio::net::TcpStream) -> Option<Seen> {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let end = loop {
            let n = socket.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
        };
        let head = String::from_utf8_lossy(&buf[..end]).to_string();
        let mut lines = head.split("\r\n");
        let mut first = lines.next()?.split(' ');
        let method = first.next()?.to_string();
        let path = first.next()?.to_string();
        let headers: Vec<(String, String)> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        let length = headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = buf[end + 4..].to_vec();
        while body.len() < length {
            let n = socket.read(&mut chunk).await.ok()?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
        }
        Some(Seen {
            method,
            path,
            headers,
            body: String::from_utf8_lossy(&body).to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::state;
    use stub::Stub;

    fn cloudflare(name: &str) -> NewProvider {
        NewProvider {
            name: name.into(),
            kind: Kind::Cloudflare,
            credentials: serde_json::json!({ "token": "cf-secret-token" }),
        }
    }

    async fn cloudflare_stub(allow: bool) -> Stub {
        Stub::serve(move |req| match (req.method.as_str(), allow) {
            (_, false) => (
                403,
                r#"{"success":false,"errors":[{"message":"no"}]}"#.into(),
            ),
            ("GET", _) if req.path.contains("name=example.com") => (
                200,
                r#"{"success":true,"result":[{"id":"z1","name":"example.com"}]}"#.into(),
            ),
            ("GET", _) => (200, r#"{"success":true,"result":[]}"#.into()),
            ("POST", _) => (200, r#"{"success":true,"result":{"id":"r1"}}"#.into()),
            _ => (200, r#"{"success":true,"result":{"id":"r1"}}"#.into()),
        })
        .await
    }

    #[test]
    fn credentials_must_carry_every_field_for_their_kind() {
        let err = Credentials::parse(Kind::Route53, &serde_json::json!({"access_key_id": "A"}))
            .err()
            .unwrap();
        assert_eq!(err.to_string(), "Enter the AWS secret access key");
        assert!(Credentials::parse(Kind::Cloudflare, &serde_json::json!({"token": " "})).is_err());
        assert!(
            Credentials::parse(
                Kind::Route53,
                &serde_json::json!({"access_key_id": "A", "secret_access_key": "S"})
            )
            .is_ok()
        );
    }

    #[tokio::test]
    async fn a_saved_provider_is_sealed_listed_without_secrets_and_refused_while_in_use() {
        let (_d, state) = state().await;
        let stub = cloudflare_stub(true).await;
        state
            .set_setting(CLOUDFLARE_API_SETTING, &stub.base)
            .await
            .unwrap();
        let http = crate::http::client();
        let saved = create(&state, &http, cloudflare("Main"), "example.com")
            .await
            .unwrap();
        assert_eq!(saved.kind, Kind::Cloudflare);
        let paths: Vec<String> = stub
            .seen()
            .iter()
            .map(|s| format!("{} {}", s.method, s.path))
            .collect();
        assert!(
            paths.contains(&"POST /zones/z1/dns_records".to_string()),
            "{paths:?}"
        );
        assert!(
            paths.contains(&"DELETE /zones/z1/dns_records/r1".to_string()),
            "{paths:?}"
        );

        let stored: String = sqlx::query_scalar("SELECT credentials FROM dns_providers")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert!(secrets::is_encrypted(&stored));
        assert!(!stored.contains("cf-secret-token"));
        assert!(
            !serde_json::to_string(&list(&state).await.unwrap())
                .unwrap()
                .contains("cf-secret-token")
        );

        let dup = create(&state, &http, cloudflare("Main"), "example.com")
            .await
            .unwrap_err();
        assert!(dup.to_string().contains("already exists"), "{dup}");

        client(&state, &http, &saved.id).await.unwrap();

        let app = crate::apps::create(
            &state,
            crate::apps::tests::new_app("ledger", &[("/", "main", false)]),
        )
        .await
        .unwrap();
        sqlx::query("UPDATE app_domains SET dns_provider_id = ? WHERE app_id = ?")
            .bind(&saved.id)
            .bind(&app.id)
            .execute(&state.pool)
            .await
            .unwrap();
        let refused = remove(&state, &saved.id).await.unwrap_err();
        assert!(
            matches!(refused.downcast_ref(), Some(ProviderError::InUse(_))),
            "{refused}"
        );
        assert!(
            refused.to_string().contains("ledger.example.com"),
            "{refused}"
        );

        sqlx::query("UPDATE app_domains SET dns_provider_id = NULL")
            .execute(&state.pool)
            .await
            .unwrap();
        remove(&state, &saved.id).await.unwrap();
        assert!(list(&state).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_token_the_provider_refuses_is_not_saved() {
        let (_d, state) = state().await;
        let stub = cloudflare_stub(false).await;
        state
            .set_setting(CLOUDFLARE_API_SETTING, &stub.base)
            .await
            .unwrap();
        let err = create(
            &state,
            &crate::http::client(),
            cloudflare("Main"),
            "example.com",
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err.downcast_ref(), Some(ProviderError::Invalid(_))),
            "{err}"
        );
        assert!(list(&state).await.unwrap().is_empty());
    }

    #[test]
    fn a_long_error_body_is_cut_short() {
        let long = "x ".repeat(500);
        assert!(excerpt(&long).chars().count() <= 201);
        assert_eq!(excerpt("a\n  b"), "a b");
    }
}
