mod support;

use axum::http::StatusCode;
use support::signed_in;

#[tokio::test]
async fn requesting_redis_from_an_app_injects_redis_url() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    let res = h
        .post_with_cookie("/api/apps/ledger/redis", "", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    let port = res.json["port"].as_u64().unwrap();
    assert!((20000..=29999).contains(&port));
    assert_eq!(res.json["maxmemory_mb"], 64);
    assert!(!res.text.contains("password"));

    let env = h.env_file("ledger");
    assert!(env.contains("REDIS_URL=redis://:"), "{env}");
    assert!(env.contains(&format!("@127.0.0.1:{port}/0\n")), "{env}");

    let got = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert_eq!(got.json["redis"]["port"], port);
    assert_eq!(got.json["managed"], serde_json::json!(["REDIS_URL"]));

    let calls = h.platform.calls();
    assert!(calls.contains(&"service mask redis-server".to_string()));
    assert!(calls.contains(&"install_packages redis".to_string()));
    assert!(calls.contains(&"service enable-now ferrum-redis-ledger".to_string()));
    let conf = h
        .platform
        .written("/var/lib/ferrum/redis/ledger/redis.conf")
        .unwrap();
    assert!(conf.contains("bind 127.0.0.1\n"));
    assert!(conf.contains("maxmemory-policy noeviction\n"));

    let listed = h.get_with_cookie("/api/redis", &cookie).await;
    assert_eq!(listed.json[0]["app_slug"], "ledger");
    assert_eq!(listed.json[0]["port"], port);

    let again = h
        .post_with_cookie("/api/apps/ledger/redis", "", &cookie)
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.json);
}

#[tokio::test]
async fn memory_is_taken_from_the_body_and_checked() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    let tiny = h
        .post_with_cookie("/api/apps/ledger/redis", r#"{"maxmemory_mb":4}"#, &cookie)
        .await;
    assert_eq!(tiny.status, StatusCode::BAD_REQUEST, "{}", tiny.json);
    let res = h
        .post_with_cookie("/api/apps/ledger/redis", r#"{"maxmemory_mb":128}"#, &cookie)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["maxmemory_mb"], 128);
    assert!(
        h.platform
            .written("/var/lib/ferrum/redis/ledger/redis.conf")
            .unwrap()
            .contains("maxmemory 128mb\n")
    );
}

#[tokio::test]
async fn releasing_redis_removes_the_url_and_the_unit() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    h.post_with_cookie("/api/apps/ledger/redis", "", &cookie)
        .await;
    let res = h
        .delete_with_cookie("/api/apps/ledger/redis", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.json);
    assert!(!h.env_file("ledger").contains("REDIS_URL="));
    assert!(
        h.platform
            .removed("/etc/systemd/system/ferrum-redis-ledger.service")
    );
    let got = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert_eq!(got.json["redis"], serde_json::Value::Null);
    assert_eq!(got.json["managed"], serde_json::json!([]));
    let again = h
        .delete_with_cookie("/api/apps/ledger/redis", &cookie)
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn deleting_the_app_removes_its_redis_but_keeps_the_database() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    h.platform.set_postgres_major(18);
    h.post_with_cookie(
        "/api/databases",
        r#"{"name":"ledger_prod","app_slug":"ledger"}"#,
        &cookie,
    )
    .await;
    h.post_with_cookie("/api/apps/ledger/redis", "", &cookie)
        .await;
    let res = h
        .delete_json_with_cookie("/api/apps/ledger", r#"{"name":"ledger"}"#, &cookie)
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.json);
    let calls = h.platform.calls();
    assert!(calls.contains(&"service stop ferrum-redis-ledger".to_string()));
    assert!(calls.contains(&"remove_tree /var/lib/ferrum/redis/ledger".to_string()));
    assert!(!h.platform.sql().iter().any(|s| s.contains("DROP")));
    let listed = h.get_with_cookie("/api/databases", &cookie).await;
    assert_eq!(listed.json[0]["name"], "ledger_prod");
    assert_eq!(listed.json[0]["linked_apps"], serde_json::json!([]));
    assert_eq!(
        h.get_with_cookie("/api/redis", &cookie).await.json,
        serde_json::json!([])
    );
}

#[tokio::test]
async fn a_host_that_refuses_redis_answers_with_the_reason() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    h.platform.fail_next("service enable-now ferrum-redis");
    let res = h
        .post_with_cookie("/api/apps/ledger/redis", "", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.json);
    let got = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert_eq!(got.json["redis"], serde_json::Value::Null);
}
