use axum::body::Body;
use axum::http::{Request, StatusCode};
use ferrum_core::state::State;
use tower::ServiceExt;

async fn app() -> axum::Router {
    let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let state = State::open(dir.path()).await.unwrap();
    ferrum::server::app(state)
}

#[tokio::test]
async fn version_endpoint_returns_build_metadata() {
    let res = app()
        .await
        .oneshot(
            Request::builder()
                .uri("/api/version")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    assert!(json["build_id"].is_string());
    assert!(json["commit_sha"].is_string());
    assert!(json["arch"].is_string());
}

#[tokio::test]
async fn unknown_path_is_404() {
    let res = app()
        .await
        .oneshot(
            Request::builder()
                .uri("/api/nope")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}
