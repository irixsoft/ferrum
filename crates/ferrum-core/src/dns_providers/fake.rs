use super::{DnsProvider, Pending, TxtHandle};
use anyhow::bail;
use std::sync::{Arc, Mutex};

/// Holds records in memory; `failing` makes every create fail with that sentence.
#[derive(Clone, Default)]
pub struct Memory {
    records: Arc<Mutex<Vec<(String, String)>>>,
    failing: Option<String>,
}

impl Memory {
    pub fn failing(sentence: &str) -> Self {
        Self {
            failing: Some(sentence.to_string()),
            ..Self::default()
        }
    }

    pub fn records(&self) -> Vec<(String, String)> {
        self.records.lock().expect("not poisoned").clone()
    }
}

impl DnsProvider for Memory {
    fn create_txt<'a>(&'a self, fqdn: &'a str, value: &'a str) -> Pending<'a, TxtHandle> {
        Box::pin(async move {
            if let Some(sentence) = &self.failing {
                bail!("{sentence}");
            }
            self.records
                .lock()
                .expect("not poisoned")
                .push((fqdn.to_string(), value.to_string()));
            Ok(TxtHandle {
                zone: "memory".into(),
                record: fqdn.to_string(),
                fqdn: fqdn.to_string(),
                value: value.to_string(),
            })
        })
    }

    fn delete_txt<'a>(&'a self, handle: TxtHandle) -> Pending<'a, ()> {
        Box::pin(async move {
            self.records
                .lock()
                .expect("not poisoned")
                .retain(|(f, v)| !(f == &handle.fqdn && v == &handle.value));
            Ok(())
        })
    }
}

/// Serves the record from Pebble's challenge test server through its management API.
pub struct ChallTestSrv {
    http: reqwest::Client,
    base: String,
}

impl ChallTestSrv {
    pub fn new(base: &str) -> Self {
        Self {
            http: crate::http::client(),
            base: base.trim_end_matches('/').to_string(),
        }
    }

    async fn post(&self, path: &str, body: serde_json::Value) -> anyhow::Result<()> {
        let res = self
            .http
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await?;
        if !res.status().is_success() {
            bail!("challtestsrv answered {} on {path}", res.status());
        }
        Ok(())
    }
}

impl DnsProvider for ChallTestSrv {
    fn create_txt<'a>(&'a self, fqdn: &'a str, value: &'a str) -> Pending<'a, TxtHandle> {
        Box::pin(async move {
            let host = format!("{}.", fqdn.trim_end_matches('.'));
            self.post(
                "/set-txt",
                serde_json::json!({ "host": host, "value": value }),
            )
            .await?;
            Ok(TxtHandle {
                zone: String::new(),
                record: host,
                fqdn: fqdn.to_string(),
                value: value.to_string(),
            })
        })
    }

    fn delete_txt<'a>(&'a self, handle: TxtHandle) -> Pending<'a, ()> {
        Box::pin(async move {
            self.post("/clear-txt", serde_json::json!({ "host": handle.record }))
                .await
        })
    }
}
