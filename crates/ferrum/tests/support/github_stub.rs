#![allow(dead_code)]

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

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
pub const BUN_LATEST: &str = "1.2.3";
const PER_PAGE: usize = 2;

#[derive(Default)]
struct Repos {
    files: HashMap<String, Vec<(String, String)>>,
    truncated: Vec<String>,
    fetched: Vec<String>,
    release: Option<Value>,
}

#[derive(Clone)]
struct Counters {
    mints: Arc<AtomicUsize>,
    expires_in: Arc<AtomicI64>,
    repo_pages: Arc<AtomicUsize>,
    installed: Arc<AtomicUsize>,
    repos: Arc<Mutex<Repos>>,
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
            repos: Arc::new(Mutex::new(Repos::default())),
        };

        let app = Router::new()
            .route("/app/installations", get(installations))
            .route("/app/installations/{id}/access_tokens", post(access_tokens))
            .route("/installation/repositories", get(repositories))
            .route("/repos/{owner}/{repo}/git/trees/{git_ref}", get(tree))
            .route("/repos/{owner}/{repo}/contents/{*path}", get(contents))
            .route("/repos/oven-sh/bun/releases/latest", get(bun_latest))
            .route("/repos/{owner}/{repo}/commits/{*git_ref}", get(commit))
            .route("/repos/{owner}/{repo}/releases/latest", get(latest_release))
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

    pub fn serve_repo(&self, full_name: &str, files: &[(&str, &str)]) {
        self.counters.repos.lock().unwrap().files.insert(
            full_name.to_string(),
            files
                .iter()
                .map(|(p, c)| (p.to_string(), c.to_string()))
                .collect(),
        );
    }

    pub fn serve_truncated_tree(&self, full_name: &str) {
        self.counters
            .repos
            .lock()
            .unwrap()
            .truncated
            .push(full_name.to_string());
    }

    pub fn contents_fetched(&self) -> Vec<String> {
        self.counters.repos.lock().unwrap().fetched.clone()
    }

    /// What `releases/latest` answers from now on, for every repository.
    pub fn set_release(&self, release: Value) {
        self.counters.repos.lock().unwrap().release = Some(release);
    }
}

/// A release shaped like GitHub's, with its assets served from `downloads`.
pub fn release_json(downloads: &str, tag: &str, name: &str, body: &str, size: u64) -> Value {
    let asset = |file: &str, size: u64| {
        json!({
            "name": file,
            "browser_download_url": format!("{downloads}/{tag}/{file}"),
            "size": size,
            "content_type": "application/octet-stream",
        })
    };
    json!({
        "tag_name": tag,
        "name": name,
        "body": body,
        "html_url": format!("https://github.com/irixsoft/ferrum/releases/tag/{tag}"),
        "published_at": "2026-09-03T10:00:00Z",
        "prerelease": false,
        "assets": [
            asset("ferrum-x86_64-unknown-linux-musl", size),
            asset("ferrum-aarch64-unknown-linux-musl", size),
            asset("SHA256SUMS", 250),
            asset("SHA256SUMS.sig", 64),
        ],
    })
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

/// GitHub answers errors with a JSON body, and octocrab refuses to classify one without it.
fn not_found() -> (StatusCode, Json<Value>) {
    (
        StatusCode::NOT_FOUND,
        Json(
            json!({ "message": "Not Found", "documentation_url": "https://docs.github.com/rest" }),
        ),
    )
}

async fn tree(
    State(c): State<Counters>,
    Path((owner, repo, git_ref)): Path<(String, String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let full_name = format!("{owner}/{repo}");
    let repos = c.repos.lock().unwrap();
    if repos.truncated.contains(&full_name) {
        return Ok(Json(json!({ "sha": "t", "tree": [], "truncated": true })));
    }
    if git_ref == "missing" {
        return Err(not_found());
    }
    let files = repos.files.get(&full_name).ok_or_else(not_found)?;
    let entries: Vec<Value> = files
        .iter()
        .map(|(path, contents)| {
            json!({ "path": path, "mode": "100644", "type": "blob", "sha": "x", "size": contents.len() })
        })
        .collect();
    Ok(Json(
        json!({ "sha": "t", "tree": entries, "truncated": false }),
    ))
}

async fn contents(
    State(c): State<Counters>,
    Path((owner, repo, path)): Path<(String, String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let full_name = format!("{owner}/{repo}");
    let mut repos = c.repos.lock().unwrap();
    repos.fetched.push(path.clone());
    let files = repos.files.get(&full_name).ok_or_else(not_found)?;
    let (_, body) = files
        .iter()
        .find(|(p, _)| *p == path)
        .ok_or_else(not_found)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(body.as_bytes());
    let wrapped: String = encoded
        .as_bytes()
        .chunks(60)
        .map(|c| format!("{}\n", String::from_utf8_lossy(c)))
        .collect();
    Ok(Json(json!({
        "type": "file",
        "encoding": "base64",
        "size": body.len(),
        "name": path.rsplit('/').next().unwrap_or(&path),
        "path": path,
        "content": wrapped,
    })))
}

async fn bun_latest() -> Json<Value> {
    Json(json!({ "tag_name": format!("bun-v{BUN_LATEST}"), "name": format!("Bun v{BUN_LATEST}") }))
}

pub const HEAD_SHA: &str = "a3f9c2d4e81b06f5c9a2f0e1d2c3b4a5968778e9";
pub const HEAD_MESSAGE: &str = "Add reconciliation window to statement export";
pub const LATEST_TAG: &str = "v1.4.0";

/// Every branch and tag but `missing` resolves to one commit; the sha carries the ref so a
/// test can see which one was asked for.
async fn commit(
    Path((_owner, _repo, git_ref)): Path<(String, String, String)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if git_ref == "missing" {
        return Err(not_found());
    }
    let sha = if git_ref == "main" {
        HEAD_SHA.to_string()
    } else {
        let mut sha: String = git_ref
            .chars()
            .filter(char::is_ascii_hexdigit)
            .collect::<String>()
            .to_ascii_lowercase();
        sha.push_str(&"0".repeat(40));
        sha.truncate(40);
        sha
    };
    Ok(Json(json!({
        "sha": sha,
        "commit": {
            "message": format!("{HEAD_MESSAGE}\n\nLonger body that the panel never shows."),
            "author": { "name": "Saeed Sakib" }
        },
        "author": { "login": "saeed" }
    })))
}

async fn latest_release(State(c): State<Counters>) -> Json<Value> {
    let set = c.repos.lock().unwrap().release.clone();
    Json(set.unwrap_or_else(|| json!({ "tag_name": LATEST_TAG, "name": LATEST_TAG })))
}
