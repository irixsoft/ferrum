mod support;

use axum::http::StatusCode;
use ferrum_core::github::webhook::sign;
use support::{
    Harness, ORG_WEBHOOK_SECRET, WEBHOOK_SECRET as SECRET, harness, push_payload, release_payload,
};

async fn connected() -> Harness {
    let h = harness().await;
    h.connect_github().await;
    h
}

#[tokio::test]
async fn a_correctly_signed_delivery_is_accepted_and_recorded() {
    let h = connected().await;
    let body = push_payload("irixsoft/ledger", "refs/heads/main", "abc123");
    let res = h.webhook("push", "d-1", &sign(SECRET, &body), &body).await;

    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let rows = h.deliveries().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].repository, "irixsoft/ledger");
    assert_eq!(rows[0].git_ref.as_deref(), Some("refs/heads/main"));
    assert_eq!(rows[0].commit_sha.as_deref(), Some("abc123"));
    assert_eq!(rows[0].event, "push");
}

#[tokio::test]
async fn a_release_from_an_older_app_registration_is_acknowledged_and_ignored() {
    let h = connected().await;
    let body = release_payload("irixsoft/ledger", "v1.2.0");

    assert_eq!(
        h.webhook("release", "d-r", &sign(SECRET, &body), &body)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert!(
        h.deliveries().await.is_empty(),
        "the tag's own push is what deploys"
    );
}

#[tokio::test]
async fn an_unsigned_or_wrongly_signed_delivery_is_refused_and_not_recorded() {
    let h = connected().await;
    let body = push_payload("irixsoft/ledger", "refs/heads/main", "abc123");

    for signature in ["", "sha256=deadbeef", &sign("the-wrong-secret", &body)] {
        let res = h.webhook("push", "d-1", signature, &body).await;
        assert_eq!(res.status, StatusCode::UNAUTHORIZED, "accepted {signature}");
    }
    assert!(
        h.deliveries().await.is_empty(),
        "an unverified body was written to the database"
    );
}

#[tokio::test]
async fn a_tampered_body_fails_even_with_a_valid_signature_for_the_original() {
    let h = connected().await;
    let original = push_payload("irixsoft/ledger", "refs/heads/main", "abc123");
    let signature = sign(SECRET, &original);
    let tampered = push_payload("attacker/evil", "refs/heads/main", "abc123");

    assert_eq!(
        h.webhook("push", "d-1", &signature, &tampered).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert!(h.deliveries().await.is_empty());
}

#[tokio::test]
async fn a_redelivery_of_the_same_id_is_accepted_but_recorded_once() {
    let h = connected().await;
    let body = push_payload("irixsoft/ledger", "refs/heads/main", "abc123");
    let signature = sign(SECRET, &body);

    for _ in 0..2 {
        assert_eq!(
            h.webhook("push", "d-1", &signature, &body).await.status,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        h.deliveries().await.len(),
        1,
        "github retries deliveries; a retry must not become a second deploy"
    );
}

#[tokio::test]
async fn a_ping_is_answered_so_github_shows_the_hook_as_healthy() {
    let h = connected().await;
    let body = br#"{"zen":"Design for failure."}"#.to_vec();

    assert_eq!(
        h.webhook("ping", "d-ping", &sign(SECRET, &body), &body)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert!(
        h.deliveries().await.is_empty(),
        "a ping is not something to act on later"
    );
}

#[tokio::test]
async fn an_event_ferrum_does_not_track_is_acknowledged_and_dropped() {
    let h = connected().await;
    let body = br#"{"action":"opened"}"#.to_vec();

    assert_eq!(
        h.webhook("issues", "d-i", &sign(SECRET, &body), &body)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert!(h.deliveries().await.is_empty());
}

#[tokio::test]
async fn a_delivery_signed_by_any_connected_app_is_accepted() {
    let h = connected().await;
    h.connect_org_github().await;
    let body = push_payload("acme/site", "refs/tags/v1.0.0", "abc123");

    assert_eq!(
        h.webhook("push", "d-org", &sign(ORG_WEBHOOK_SECRET, &body), &body)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.webhook("push", "d-bad", &sign("whsec_other", &body), &body)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let rows = h.deliveries().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].repository, "acme/site");
}

#[tokio::test]
async fn a_delivery_arriving_with_no_connection_is_refused() {
    let h = harness().await;
    let body = push_payload("irixsoft/ledger", "refs/heads/main", "abc123");

    assert_eq!(
        h.webhook("push", "d-1", &sign(SECRET, &body), &body)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_delivery_after_disconnecting_is_refused() {
    let h = connected().await;
    let body = push_payload("irixsoft/ledger", "refs/heads/main", "abc123");
    ferrum_core::github::disconnect(&h.db, 12345).await.unwrap();

    assert_eq!(
        h.webhook("push", "d-1", &sign(SECRET, &body), &body)
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn the_webhook_is_the_only_public_write_endpoint() {
    let h = harness().await;
    for route in [
        "/api/users",
        "/api/tokens",
        "/api/sessions",
        "/api/github/repos",
        "/api/github/connect",
    ] {
        assert_eq!(
            h.post(route, "{}").await.status,
            StatusCode::UNAUTHORIZED,
            "{route}"
        );
    }
}
