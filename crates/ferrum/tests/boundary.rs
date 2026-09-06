mod support;

use axum::http::StatusCode;
use ferrum_core::tokens;
use support::*;

#[tokio::test]
async fn an_unauthenticated_request_is_401_not_404() {
    let h = harness().await;
    assert_eq!(h.get("/api/me").await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn health_and_version_stay_public() {
    let h = harness().await;
    assert_eq!(h.get("/api/health").await.status, StatusCode::OK);
    assert_eq!(h.get("/api/version").await.status, StatusCode::OK);
}

#[tokio::test]
async fn a_session_cookie_authenticates() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    let cookie = h.register(&mut key, &link).await.session_cookie().unwrap();

    let res = h.get_with_cookie("/api/me", &cookie).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["kind"], "user");
    assert_eq!(res.json["name"], "Saeed");
    assert_eq!(res.json["read_only"], false);
}

#[tokio::test]
async fn a_bearer_token_authenticates_as_a_machine() {
    let h = harness().await;
    let minted = tokens::mint(&h.db, "agent", true).await.unwrap();

    let res = h.get_with_bearer("/api/me", &minted.secret).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.json["kind"], "machine");
    assert_eq!(res.json["name"], "agent");
    assert_eq!(res.json["read_only"], true);
}

#[tokio::test]
async fn a_writing_token_is_not_read_only() {
    let h = harness().await;
    let minted = tokens::mint(&h.db, "deployer", false).await.unwrap();
    let res = h.get_with_bearer("/api/me", &minted.secret).await;
    assert_eq!(res.json["read_only"], false);
}

#[tokio::test]
async fn a_revoked_token_is_401_immediately() {
    let h = harness().await;
    let minted = tokens::mint(&h.db, "agent", false).await.unwrap();
    assert_eq!(
        h.get_with_bearer("/api/me", &minted.secret).await.status,
        StatusCode::OK
    );

    tokens::revoke(&h.db, &minted.token.id).await.unwrap();
    assert_eq!(
        h.get_with_bearer("/api/me", &minted.secret).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_bogus_cookie_does_not_authenticate() {
    let h = harness().await;
    assert_eq!(
        h.get_with_cookie("/api/me", "hello").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_bogus_bearer_token_does_not_authenticate() {
    let h = harness().await;
    assert_eq!(
        h.get_with_bearer("/api/me", "ferr_nonsense").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_signed_out_session_stops_working() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    let cookie = h.register(&mut key, &link).await.session_cookie().unwrap();

    h.post_with_cookie("/api/auth/logout", "", &cookie).await;

    assert_eq!(
        h.get_with_cookie("/api/me", &cookie).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_expired_session_stops_working() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    let cookie = h.register(&mut key, &link).await.session_cookie().unwrap();

    sqlx::query("UPDATE sessions SET expires_at = datetime('now', '-1 day')")
        .execute(&h.db.pool)
        .await
        .unwrap();

    assert_eq!(
        h.get_with_cookie("/api/me", &cookie).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_unauthenticated_error_is_json() {
    let h = harness().await;
    let res = h.get("/api/me").await;
    assert!(
        res.json["error"].is_string(),
        "the panel needs a message to show: {}",
        res.json
    );
}

#[tokio::test]
async fn a_read_only_token_reads_everywhere_and_writes_nowhere() {
    let h = harness().await;
    let token = h.machine_token(true).await;

    for route in ["/api/me", "/api/users", "/api/tokens", "/api/github/status"] {
        assert_eq!(
            h.get_with_bearer(route, &token).await.status,
            StatusCode::OK,
            "{route}"
        );
    }

    for route in ["/api/users", "/api/tokens", "/api/github/connect"] {
        assert_eq!(
            h.post_with_bearer(route, "{}", &token).await.status,
            StatusCode::FORBIDDEN,
            "a read-only token wrote to {route}"
        );
    }
    assert_eq!(
        h.delete_with_bearer("/api/tokens/any", &token).await.status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn a_writing_token_is_not_stopped_by_the_read_only_check() {
    let h = harness().await;
    let token = h.machine_token(false).await;
    assert_eq!(
        h.post_with_bearer("/api/users", r#"{"name":"Teammate"}"#, &token)
            .await
            .status,
        StatusCode::OK
    );
}
