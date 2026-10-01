use super::{DnsProvider, Pending, TxtHandle, excerpt};
use anyhow::bail;
use reqwest::{Method, StatusCode};
use serde::Deserialize;
use serde_json::Value;

pub struct Cloudflare {
    http: reqwest::Client,
    base: String,
    token: String,
}

#[derive(Deserialize)]
struct Zone {
    id: String,
}

#[derive(Deserialize)]
struct Record {
    id: String,
}

impl Cloudflare {
    pub fn new(http: reqwest::Client, base: String, token: String) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            token,
        }
    }

    async fn call(&self, method: Method, path: &str, body: Option<Value>) -> anyhow::Result<Value> {
        let mut req = self
            .http
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(body) = body {
            req = req.json(&body);
        }
        let res = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Cloudflare could not be reached: {e}"))?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        match status {
            s if s.is_success() => Ok(serde_json::from_str(&text)?),
            StatusCode::UNAUTHORIZED => {
                bail!(
                    "Cloudflare refused the API token; check it was copied whole and has not been rolled"
                )
            }
            StatusCode::FORBIDDEN => {
                bail!(
                    "The Cloudflare token may not edit DNS here; give it Zone → DNS → Edit on this zone"
                )
            }
            StatusCode::NOT_FOUND => {
                bail!("Cloudflare does not know that zone or record, or the token cannot see it")
            }
            s => bail!("Cloudflare answered {s}: {}", message(&text)),
        }
    }

    async fn zone_of(&self, fqdn: &str) -> anyhow::Result<String> {
        let labels: Vec<&str> = fqdn.trim_end_matches('.').split('.').collect();
        for i in 0..labels.len().saturating_sub(1) {
            let candidate = labels[i..].join(".");
            let found = self
                .call(Method::GET, &format!("/zones?name={candidate}"), None)
                .await?;
            let zones: Vec<Zone> = serde_json::from_value(found["result"].clone())?;
            if let Some(zone) = zones.into_iter().next() {
                return Ok(zone.id);
            }
        }
        bail!("The Cloudflare token cannot see a zone that holds {fqdn}")
    }
}

fn message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["errors"][0]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| excerpt(body))
}

impl DnsProvider for Cloudflare {
    fn create_txt<'a>(&'a self, fqdn: &'a str, value: &'a str) -> Pending<'a, TxtHandle> {
        Box::pin(async move {
            let zone = self.zone_of(fqdn).await?;
            let created = self
                .call(
                    Method::POST,
                    &format!("/zones/{zone}/dns_records"),
                    Some(serde_json::json!({
                        "type": "TXT",
                        "name": fqdn,
                        "content": format!("\"{value}\""),
                        "ttl": 60,
                    })),
                )
                .await?;
            let record: Record = serde_json::from_value(created["result"].clone())?;
            Ok(TxtHandle {
                zone,
                record: record.id,
                fqdn: fqdn.to_string(),
                value: value.to_string(),
            })
        })
    }

    fn delete_txt<'a>(&'a self, handle: TxtHandle) -> Pending<'a, ()> {
        Box::pin(async move {
            self.call(
                Method::DELETE,
                &format!("/zones/{}/dns_records/{}", handle.zone, handle.record),
                None,
            )
            .await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns_providers::stub::Stub;

    #[tokio::test]
    async fn walks_to_the_zone_then_creates_and_deletes_the_record() {
        let stub = Stub::serve(|req| match req.method.as_str() {
            "GET" if req.path == "/zones?name=example.com" => (
                200,
                r#"{"success":true,"result":[{"id":"zone9","name":"example.com"}]}"#.into(),
            ),
            "GET" => (200, r#"{"success":true,"result":[]}"#.into()),
            "POST" => (200, r#"{"success":true,"result":{"id":"rec7"}}"#.into()),
            _ => (200, r#"{"success":true,"result":{"id":"rec7"}}"#.into()),
        })
        .await;
        let cf = Cloudflare::new(crate::http::client(), stub.base.clone(), "tok".into());
        let handle = cf
            .create_txt("_acme-challenge.app.example.com", "v4lue")
            .await
            .unwrap();
        assert_eq!(handle.zone, "zone9");
        assert_eq!(handle.record, "rec7");
        cf.delete_txt(handle).await.unwrap();

        let seen = stub.seen();
        let lines: Vec<String> = seen
            .iter()
            .map(|s| format!("{} {}", s.method, s.path))
            .collect();
        assert_eq!(
            lines,
            [
                "GET /zones?name=_acme-challenge.app.example.com",
                "GET /zones?name=app.example.com",
                "GET /zones?name=example.com",
                "POST /zones/zone9/dns_records",
                "DELETE /zones/zone9/dns_records/rec7",
            ]
        );
        assert!(
            seen.iter()
                .all(|s| s.header("authorization") == Some("Bearer tok"))
        );
        let body: Value = serde_json::from_str(&seen[3].body).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "type": "TXT",
                "name": "_acme-challenge.app.example.com",
                "content": "\"v4lue\"",
                "ttl": 60,
            })
        );
    }

    #[tokio::test]
    async fn a_refused_token_says_so() {
        let stub = Stub::serve(|_| (401, r#"{"success":false}"#.into())).await;
        let cf = Cloudflare::new(crate::http::client(), stub.base.clone(), "tok".into());
        let err = cf
            .create_txt("_acme-challenge.example.com", "v")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("refused the API token"), "{err}");

        let stub = Stub::serve(|_| (403, "{}".into())).await;
        let cf = Cloudflare::new(crate::http::client(), stub.base.clone(), "tok".into());
        let err = cf
            .create_txt("_acme-challenge.example.com", "v")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("DNS → Edit"), "{err}");
    }

    #[tokio::test]
    async fn no_zone_in_the_account_is_a_sentence() {
        let stub = Stub::serve(|_| (200, r#"{"success":true,"result":[]}"#.into())).await;
        let cf = Cloudflare::new(crate::http::client(), stub.base.clone(), "tok".into());
        let err = cf
            .create_txt("_acme-challenge.example.com", "v")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot see a zone"), "{err}");
    }
}
