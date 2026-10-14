mod support;

use axum::http::StatusCode;
use support::signed_in;

const CUSTOM: &str = "/etc/nginx/ferrum-custom/ledger.conf";

#[tokio::test]
async fn the_managed_file_is_read_and_the_custom_one_is_edited_behind_nginx_t() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    let res = h.get_with_cookie("/api/apps/ledger/nginx", &cookie).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let managed = res.json["managed"].as_str().unwrap();
    assert!(managed.starts_with("# managed by Ferrum"), "{managed}");
    assert!(managed.contains("server_name ledger.example.com;"));
    assert!(managed.contains("include /etc/nginx/ferrum-custom/ledger.conf;"));
    assert_eq!(res.json["custom"], "");

    let before = h.platform.calls().len();
    let body = r#"{"custom":"location /downloads/ {\n  alias /var/lib/ferrum/apps/ledger/shared/storage/;\n}\n"}"#;
    let saved = h
        .put_with_cookie("/api/apps/ledger/nginx", body, &cookie)
        .await;
    assert_eq!(saved.status, StatusCode::NO_CONTENT, "{}", saved.json);
    let written = h.platform.written(CUSTOM).unwrap();
    assert!(written.starts_with("location /downloads/ {"), "{written}");
    let calls = h.platform.calls()[before..].to_vec();
    let wrote = calls
        .iter()
        .position(|c| c == &format!("write_file {CUSTOM} 644"))
        .unwrap();
    let tested = calls.iter().position(|c| c == "nginx_test").unwrap();
    let reloaded = calls
        .iter()
        .position(|c| c == "service reload nginx")
        .unwrap();
    assert!(wrote < tested && tested < reloaded, "{calls:#?}");
    assert_eq!(
        h.get_with_cookie("/api/apps/ledger/nginx", &cookie)
            .await
            .json["custom"],
        written
    );

    let before = h.platform.calls().len();
    h.platform.fail_next("nginx_test");
    let rejected = h
        .put_with_cookie(
            "/api/apps/ledger/nginx",
            r#"{"custom":"locaton / {}"}"#,
            &cookie,
        )
        .await;
    assert_eq!(
        rejected.status,
        StatusCode::BAD_REQUEST,
        "{}",
        rejected.json
    );
    assert_eq!(
        rejected.json["error"],
        "nginx rejected the file: scripted failure"
    );
    assert_eq!(
        h.platform.written(CUSTOM).unwrap(),
        written,
        "the previous file is back"
    );
    let calls = h.platform.calls()[before..].to_vec();
    assert!(
        !calls.iter().any(|c| c == "service reload nginx"),
        "{calls:#?}"
    );

    assert_eq!(
        h.get_with_cookie("/api/apps/nope/nginx", &cookie)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let token = h.machine_token(true).await;
    assert_eq!(
        h.get_with_bearer("/api/apps/ledger/nginx", &token)
            .await
            .status,
        StatusCode::OK
    );
    let refused = h
        .send(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/api/apps/ledger/nginx")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                .body(axum::body::Body::from(r#"{"custom":""}"#))
                .unwrap(),
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_custom_file_that_reaches_the_host_is_refused_before_anything_is_written() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    let before = h.platform.calls().len();
    for (custom, sentence) in [
        (
            "location /x {\n  access_log \"/etc/cron.d/x\";\n}\n",
            "The custom file cannot use `access_log`; nginx's logs are Ferrum's.",
        ),
        (
            "root /;",
            "`root` must stay under /var/lib/ferrum/apps/ledger/.",
        ),
    ] {
        let res = h
            .put_with_cookie(
                "/api/apps/ledger/nginx",
                &serde_json::json!({ "custom": custom }).to_string(),
                &cookie,
            )
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.json);
        assert_eq!(res.json["error"], sentence);
    }
    assert_eq!(h.platform.written(CUSTOM).as_deref(), Some(""));
    let calls = h.platform.calls()[before..].to_vec();
    assert!(
        !calls
            .iter()
            .any(|c| c.contains(CUSTOM) || c == "nginx_test"),
        "{calls:#?}"
    );
}
