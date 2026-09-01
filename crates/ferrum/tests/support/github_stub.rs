#![allow(dead_code)]

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

/// A 2048-bit key generated once per test binary. Committing a PEM would put a private key in
/// the repository, and `EncodingKey::from_rsa_pem` rejects anything that is not a real one.
pub static TEST_KEY: LazyLock<String> = LazyLock::new(|| {
    use rsa::pkcs1::EncodeRsaPrivateKey;
    let mut rng = rsa::rand_core::OsRng;
    rsa::RsaPrivateKey::new(&mut rng, 2048)
        .expect("generating a test RSA key")
        .to_pkcs1_pem(rsa::pkcs1::LineEnding::LF)
        .expect("encoding the test RSA key")
        .to_string()
});

pub const INSTALLATION_ID: i64 = 4242;
const PER_PAGE: usize = 2;

#[derive(Clone)]
struct Counters {
    mints: Arc<AtomicUsize>,
    expires_in: Arc<AtomicI64>,
    repo_pages: Arc<AtomicUsize>,
    installed: Arc<AtomicUsize>,
}

pub struct StubGithub {
    pub base: String,
    counters: Counters,
}

impl StubGithub {
    pub async fn start() -> Self {
        let counters = Counters {
            mints: Arc::new(AtomicUsize::new(0)),
            expires_in: Arc::new(AtomicI64::new(3600)),
            repo_pages: Arc::new(AtomicUsize::new(0)),
            installed: Arc::new(AtomicUsize::new(1)),
        };

        let app = Router::new()
            .route("/app/installations", get(installations))
            .route("/app/installations/{id}/access_tokens", post(access_tokens))
            .route("/installation/repositories", get(repositories))
            .with_state(counters.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self {
            base: format!("http://{addr}"),
            counters,
        }
    }

    pub fn mint_calls(&self) -> usize {
        self.counters.mints.load(Ordering::SeqCst)
    }

    pub fn repo_page_calls(&self) -> usize {
        self.counters.repo_pages.load(Ordering::SeqCst)
    }

    pub fn tokens_expire_in(&self, seconds: i64) {
        self.counters.expires_in.store(seconds, Ordering::SeqCst);
    }

    pub fn uninstall(&self) {
        self.counters.installed.store(0, Ordering::SeqCst);
    }
}

async fn installations(State(c): State<Counters>) -> Json<Value> {
    if c.installed.load(Ordering::SeqCst) == 0 {
        return Json(json!([]));
    }
    Json(json!([{ "id": INSTALLATION_ID }]))
}

async fn access_tokens(State(c): State<Counters>, Path(_id): Path<i64>) -> Json<Value> {
    let nth = c.mints.fetch_add(1, Ordering::SeqCst) + 1;
    let seconds = c.expires_in.load(Ordering::SeqCst);
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(seconds);

    Json(json!({
        "token": format!("ghs_stub_{nth}"),
        "expires_at": expires_at.to_rfc3339(),
        "permissions": { "contents": "read", "metadata": "read" },
    }))
}

#[derive(Deserialize)]
struct Paging {
    page: Option<usize>,
}

async fn repositories(State(c): State<Counters>, Query(paging): Query<Paging>) -> Json<Value> {
    c.repo_pages.fetch_add(1, Ordering::SeqCst);

    let all = [
        ("irixsoft/ledger", false, "main"),
        ("irixsoft/panel", true, "main"),
        ("irixsoft/notes", true, "trunk"),
    ];
    let page = paging.page.unwrap_or(1).max(1);
    let start = (page - 1) * PER_PAGE;

    let repositories: Vec<Value> = all
        .iter()
        .skip(start)
        .take(PER_PAGE)
        .map(|(full_name, private, default_branch)| {
            json!({
                "full_name": full_name,
                "private": private,
                "default_branch": default_branch,
                "pushed_at": "2026-08-30T10:00:00Z",
            })
        })
        .collect();

    Json(json!({ "total_count": all.len(), "repositories": repositories }))
}
