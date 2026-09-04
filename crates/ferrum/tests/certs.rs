mod support;

use axum::http::StatusCode;
use serde_json::Value;
use support::{Harness, PUBLIC_IP, new_app_json, signed_in};

async fn wait_for_kind(h: &Harness, slug: &str, cookie: &str, kind: &str) -> Value {
    for _ in 0..300 {
        let res = h
            .get_with_cookie(&format!("/api/apps/{slug}"), cookie)
            .await;
        if res.json["certificates"][0]["status"]["kind"] == kind {
            return res.json["certificates"][0].clone();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("{slug} never reported a {kind} certificate");
}

#[tokio::test]
async fn a_domain_pointing_elsewhere_waits_for_dns_and_never_contacts_the_directory() {
    let (h, cookie) = signed_in().await;
    ferrum_core::setup::set_email(&h.db, "me@example.com")
        .await
        .unwrap();
    h.create_app("ledger", &cookie).await;
    let cert = wait_for_kind(&h, "ledger", &cookie, "waiting_for_dns").await;
    assert_eq!(cert["domain"], "ledger.example.com");
    let detail = cert["status"]["detail"].as_str().unwrap();
    assert!(detail.contains("no A record"), "{detail}");
    let attempts: i64 = sqlx::query_scalar("SELECT attempts FROM cert_attempts")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();
    assert_eq!(attempts, 0);
}

#[tokio::test]
async fn a_resolved_domain_is_tried_and_a_failure_backs_off_until_retried() {
    let (h, cookie) = signed_in().await;
    ferrum_core::setup::set_email(&h.db, "me@example.com")
        .await
        .unwrap();
    let json = new_app_json("ledger").replace("ledger.example.com", "resolved.example.com");
    h.create_app_from(&json, &cookie).await;
    let cert = wait_for_kind(&h, "ledger", &cookie, "failed").await;
    assert_eq!(cert["domain"], "resolved.example.com");
    assert!(cert["status"]["retry_at"].as_str().unwrap().ends_with('Z'));
    let detail = cert["status"]["detail"].as_str().unwrap();
    assert!(
        detail.starts_with("acme:"),
        "a resolved domain fails at the directory, never at a second DNS lookup: {detail}"
    );
    assert!(!detail.contains(PUBLIC_IP));

    let retry = h
        .post_with_cookie("/api/apps/ledger/certificates", "", &cookie)
        .await;
    assert_eq!(retry.status, StatusCode::ACCEPTED, "{}", retry.json);
    wait_for_kind(&h, "ledger", &cookie, "failed").await;
    assert!(
        !h.platform
            .calls()
            .iter()
            .any(|c| c.starts_with("write_file /etc/nginx") && c.contains("443")),
        "nothing was issued, so the vhost never gained TLS"
    );

    let token = h.machine_token(true).await;
    assert_eq!(
        h.post_with_bearer("/api/apps/ledger/certificates", "", &token)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}
