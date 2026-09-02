#![allow(dead_code)]

use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A real HTTP server handing out a Node-shaped tarball for any path, with a Content-Length.
pub struct StubDownloads {
    pub base: String,
    hits: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct Served {
    hits: Arc<AtomicUsize>,
    body: Arc<Vec<u8>>,
}

pub fn node_tarball() -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut tar = tar::Builder::new(gz);
    for (path, data, mode) in [
        ("node-v22.11.0-linux-x64/bin/node", &b"#!node"[..], 0o755),
        ("node-v22.11.0-linux-x64/LICENSE", &b"MIT"[..], 0o644),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        tar.append_data(&mut header, path, data).unwrap();
    }
    tar.into_inner().unwrap().finish().unwrap()
}

impl StubDownloads {
    pub async fn start() -> Self {
        let hits = Arc::new(AtomicUsize::new(0));
        let served = Served {
            hits: hits.clone(),
            body: Arc::new(node_tarball()),
        };
        let app = Router::new()
            .route("/index.json", get(index))
            .route("/{*path}", get(serve))
            .with_state(served);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base: format!("http://{addr}"),
            hits,
        }
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

/// nodejs.org's index, newest first, with one current, two LTS and one ancient release.
pub const NODE_INDEX: &str = r#"[
  {"version":"v26.8.1","date":"2026-08-26","files":["linux-arm64","linux-x64"],"lts":false},
  {"version":"v24.9.0","date":"2026-08-20","files":["linux-arm64","linux-x64"],"lts":"Krypton"},
  {"version":"v22.11.0","date":"2024-10-29","files":["linux-arm64","linux-x64"],"lts":"Jod"},
  {"version":"v0.8.0","date":"2012-06-25","files":["src"],"lts":false}
]"#;

async fn index() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/json")], NODE_INDEX)
}

async fn serve(State(s): State<Served>) -> impl IntoResponse {
    s.hits.fetch_add(1, Ordering::SeqCst);
    (
        [
            (header::CONTENT_TYPE, "application/gzip".to_string()),
            (header::CONTENT_LENGTH, s.body.len().to_string()),
        ],
        s.body.as_ref().clone(),
    )
}
