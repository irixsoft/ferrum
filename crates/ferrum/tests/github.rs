mod support;

use axum::http::StatusCode;
use support::{HOSTNAME, harness, signed_in};

#[tokio::test]
async fn the_manifest_names_this_host_and_asks_for_read_only_access() {
    let (h, cookie) = signed_in().await;
    let res = h.post_with_cookie("/api/github/connect", "", &cookie).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);

    let m = &res.json["manifest"];
    assert_eq!(
        m["hook_attributes"]["url"],
        format!("https://{HOSTNAME}/api/github/webhook")
    );
    assert_eq!(
        m["redirect_url"],
        format!("https://{HOSTNAME}/api/github/callback")
    );
    assert_eq!(m["default_permissions"]["contents"], "read");
    assert_eq!(m["public"], false);
    assert_eq!(m["default_events"], serde_json::json!(["push", "release"]));
    assert!(
        m["default_permissions"]
            .as_object()
            .unwrap()
            .values()
            .all(|v| v == "read"),
        "ferrum never writes to a repository: {m}"
    );
}

#[tokio::test]
async fn the_handoff_carries_the_state_in_the_form_action() {
    let (h, cookie) = signed_in().await;
    let res = h.post_with_cookie("/api/github/connect", "", &cookie).await;

    let state = res.json["state"].as_str().unwrap();
    assert!(!state.is_empty());
    assert_eq!(
        res.json["action"],
        format!("https://github.com/settings/apps/new?state={state}")
    );
}

#[tokio::test]
async fn connecting_requires_authentication_but_the_callback_does_not() {
    let h = harness().await;
    assert_eq!(
        h.post("/api/github/connect", "").await.status,
        StatusCode::UNAUTHORIZED
    );

    let res = h.get("/api/github/callback?code=x&state=unknown").await;
    assert_ne!(
        res.status,
        StatusCode::UNAUTHORIZED,
        "a SameSite=Strict cookie is never sent on a redirect from github.com"
    );
}

#[tokio::test]
async fn an_unknown_state_is_refused_before_the_code_is_exchanged() {
    let h = harness().await;
    let res = h
        .get("/api/github/callback?code=realcode&state=never-issued")
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert!(
        res.json["error"].as_str().unwrap().contains("expired"),
        "{}",
        res.json
    );
}

#[tokio::test]
async fn a_callback_without_a_code_does_not_consume_the_state() {
    let (h, cookie) = signed_in().await;
    let state = h.connect_state(&cookie).await;

    assert_eq!(
        h.get(&format!("/api/github/callback?state={state}"))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let res = h
        .get(&format!("/api/github/callback?code=x&state={state}"))
        .await;
    assert!(
        !res.json["error"].as_str().unwrap().contains("expired"),
        "a missing code must not burn the state: {}",
        res.json
    );
}

#[tokio::test]
async fn a_state_can_only_be_used_once() {
    let (h, cookie) = signed_in().await;
    let state = h.connect_state(&cookie).await;

    let first = h
        .get(&format!("/api/github/callback?code=x&state={state}"))
        .await;
    let second = h
        .get(&format!("/api/github/callback?code=x&state={state}"))
        .await;

    assert_eq!(second.status, StatusCode::BAD_REQUEST);
    assert!(second.json["error"].as_str().unwrap().contains("expired"));
    assert_ne!(
        first.json, second.json,
        "the first attempt got past the state check"
    );
}

#[tokio::test]
async fn status_is_honest_before_anything_is_connected() {
    let (h, cookie) = signed_in().await;
    let res = h.get_with_cookie("/api/github/status", &cookie).await;
    assert_eq!(res.json["connected"], false);
    assert!(res.json.get("app_name").is_none(), "{}", res.json);
}

#[tokio::test]
async fn status_reports_a_connection_without_leaking_its_secrets() {
    let (h, cookie) = signed_in().await;
    h.connect_github().await;

    let res = h.get_with_cookie("/api/github/status", &cookie).await;
    assert_eq!(res.json["connected"], true);
    assert_eq!(res.json["app_name"], "ferrum-panel-example");
    assert_eq!(res.json["account"], "irixsoft");
    assert!(!res.text.contains("PRIVATE KEY"), "{}", res.text);
    assert!(!res.text.contains("whsec"), "{}", res.text);
}

#[tokio::test]
async fn disconnecting_forgets_the_app() {
    let (h, cookie) = signed_in().await;
    h.connect_github().await;

    assert_eq!(
        h.delete_with_cookie("/api/github", &cookie).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.get_with_cookie("/api/github/status", &cookie).await.json["connected"],
        false
    );
}

#[tokio::test]
async fn a_read_only_token_cannot_connect_or_disconnect() {
    let h = harness().await;
    let token = h.machine_token(true).await;
    h.connect_github().await;

    assert_eq!(
        h.post_with_bearer("/api/github/connect", "", &token)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.delete_with_bearer("/api/github", &token).await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.get_with_bearer("/api/github/status", &token).await.json["connected"],
        true,
        "a read-only token still reads"
    );
}
