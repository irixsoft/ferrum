mod support;

use axum::http::StatusCode;
use support::{Harness, signed_in};

const MAJOR: u32 = 18;

async fn wait_for_install(h: &Harness, cookie: &str) -> serde_json::Value {
    for _ in 0..200 {
        let res = h.get_with_cookie("/api/postgres", cookie).await;
        if res.json["installing"] == false {
            return res.json;
        }
        tokio::task::yield_now().await;
    }
    panic!("the install never finished");
}

#[tokio::test]
async fn postgres_is_installed_in_the_background_once_and_the_status_says_so() {
    let (h, cookie) = signed_in().await;
    let before = h.get_with_cookie("/api/postgres", &cookie).await;
    assert_eq!(before.status, StatusCode::OK);
    assert_eq!(before.json["installed"], false);
    assert_eq!(before.json["major"], serde_json::Value::Null);
    assert_eq!(
        before.json["tunnel"],
        "ssh -L 5432:127.0.0.1:5432 root@panel.example.com"
    );
    assert!(before.json["extensions"].as_array().unwrap().len() == 4);

    let started = h
        .post_with_cookie("/api/postgres/install", "", &cookie)
        .await;
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", started.json);
    let done = wait_for_install(&h, &cookie).await;
    assert_eq!(done["error"], serde_json::Value::Null);
    assert_eq!(
        done["major"], MAJOR,
        "pinned even before the host reports it"
    );
    assert_eq!(
        h.platform
            .calls_matching("install_packages postgresql-")
            .len(),
        1
    );

    h.platform.set_postgres_major(MAJOR);
    let after = h.get_with_cookie("/api/postgres", &cookie).await;
    assert_eq!(after.json["installed"], true);
    assert_eq!(after.json["major"], MAJOR);

    h.post_with_cookie("/api/postgres/install", "", &cookie)
        .await;
    wait_for_install(&h, &cookie).await;
    assert_eq!(
        h.platform
            .calls_matching("install_packages postgresql-")
            .len(),
        1,
        "a second install touches no package"
    );
}

#[tokio::test]
async fn a_failed_install_is_reported_and_can_be_retried() {
    let (h, cookie) = signed_in().await;
    h.platform.fail_next("install_packages postgresql-");
    h.post_with_cookie("/api/postgres/install", "", &cookie)
        .await;
    let failed = wait_for_install(&h, &cookie).await;
    assert!(
        failed["error"]
            .as_str()
            .unwrap()
            .contains("scripted failure"),
        "{failed}"
    );
    assert_eq!(failed["installed"], false);
    h.post_with_cookie("/api/postgres/install", "", &cookie)
        .await;
    let retried = wait_for_install(&h, &cookie).await;
    assert_eq!(retried["error"], serde_json::Value::Null);
}

#[tokio::test]
async fn creating_a_database_from_an_app_links_it_and_rewrites_the_env() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    h.platform.set_postgres_major(MAJOR);
    let res = h
        .post_with_cookie(
            "/api/databases",
            r#"{"name":"ledger_prod","app_slug":"ledger","extensions":["pg_trgm"]}"#,
            &cookie,
        )
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert!(!res.text.contains("password"), "{}", res.text);
    assert_eq!(res.json["role"], "ledger_prod");
    assert_eq!(res.json["connection_limit"], 20);
    assert_eq!(res.json["linked_apps"], serde_json::json!(["ledger"]));
    assert_eq!(res.json["extensions"], serde_json::json!(["pg_trgm"]));

    let env = h.env_file("ledger");
    assert!(
        env.contains("DATABASE_URL=postgres://ledger_prod:"),
        "{env}"
    );
    assert!(env.contains("@127.0.0.1:5432/ledger_prod\n"), "{env}");

    let got = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert_eq!(got.json["databases"], serde_json::json!(["ledger_prod"]));
    assert_eq!(got.json["managed"], serde_json::json!(["DATABASE_URL"]));
    assert_eq!(got.json["redis"], serde_json::Value::Null);

    let listed = h.get_with_cookie("/api/databases", &cookie).await;
    assert_eq!(listed.json[0]["name"], "ledger_prod");
    let shown = h
        .get_with_cookie("/api/databases/ledger_prod", &cookie)
        .await;
    assert_eq!(
        shown.json["url_hint"],
        "postgres://ledger_prod:<password>@127.0.0.1:5432/ledger_prod"
    );
}

#[tokio::test]
async fn a_database_needs_postgres_first_and_a_bad_name_never_reaches_psql() {
    let (h, cookie) = signed_in().await;
    let early = h
        .post_with_cookie("/api/databases", r#"{"name":"ledger_prod"}"#, &cookie)
        .await;
    assert_eq!(early.status, StatusCode::CONFLICT, "{}", early.json);

    h.platform.set_postgres_major(MAJOR);
    for name in ["Ledger", "a;b", ""] {
        let res = h
            .post_with_cookie(
                "/api/databases",
                &serde_json::json!({ "name": name }).to_string(),
                &cookie,
            )
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST, "{name:?}");
    }
    let missing_app = h
        .post_with_cookie(
            "/api/databases",
            r#"{"name":"ok","app_slug":"ghost"}"#,
            &cookie,
        )
        .await;
    assert_eq!(missing_app.status, StatusCode::NOT_FOUND);
    assert!(h.platform.sql().is_empty(), "{:?}", h.platform.sql());
}

#[tokio::test]
async fn psql_errors_reach_the_panel_as_sentences() {
    let (h, cookie) = signed_in().await;
    h.platform.set_postgres_major(MAJOR);
    h.platform.fail_next("CREATE ROLE");
    let res = h
        .post_with_cookie("/api/databases", r#"{"name":"ledger_prod"}"#, &cookie)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.json);
    assert!(
        res.json["error"]
            .as_str()
            .unwrap()
            .starts_with("PostgreSQL refused:"),
        "{}",
        res.json
    );
    let listed = h.get_with_cookie("/api/databases", &cookie).await;
    assert_eq!(listed.json, serde_json::json!([]));
}

#[tokio::test]
async fn deleting_a_linked_database_is_refused_with_the_apps_named() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    h.platform.set_postgres_major(MAJOR);
    h.post_with_cookie("/api/databases", r#"{"name":"ledger_prod"}"#, &cookie)
        .await;
    let linked = h
        .post_with_cookie("/api/apps/ledger/databases/ledger_prod", "", &cookie)
        .await;
    assert_eq!(linked.status, StatusCode::NO_CONTENT, "{}", linked.json);
    assert!(h.env_file("ledger").contains("DATABASE_URL="));

    let refused = h
        .delete_json_with_cookie(
            "/api/databases/ledger_prod",
            r#"{"name":"ledger_prod"}"#,
            &cookie,
        )
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT, "{}", refused.json);
    assert!(refused.json["error"].as_str().unwrap().contains("ledger"));

    let unlinked = h
        .delete_with_cookie("/api/apps/ledger/databases/ledger_prod", &cookie)
        .await;
    assert_eq!(unlinked.status, StatusCode::NO_CONTENT);
    assert!(!h.env_file("ledger").contains("DATABASE_URL="));
    let again = h
        .delete_with_cookie("/api/apps/ledger/databases/ledger_prod", &cookie)
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);

    let wrong_name = h
        .delete_json_with_cookie("/api/databases/ledger_prod", r#"{"name":"nope"}"#, &cookie)
        .await;
    assert_eq!(wrong_name.status, StatusCode::BAD_REQUEST);
    let deleted = h
        .delete_json_with_cookie(
            "/api/databases/ledger_prod",
            r#"{"name":"ledger_prod"}"#,
            &cookie,
        )
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT, "{}", deleted.json);
    assert!(
        h.platform
            .sql()
            .iter()
            .any(|s| s.contains("DROP DATABASE IF EXISTS \"ledger_prod\""))
    );
    let gone = h
        .get_with_cookie("/api/databases/ledger_prod", &cookie)
        .await;
    assert_eq!(gone.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn extensions_are_enabled_from_the_list_only() {
    let (h, cookie) = signed_in().await;
    h.platform.set_postgres_major(MAJOR);
    h.post_with_cookie("/api/databases", r#"{"name":"ledger_prod"}"#, &cookie)
        .await;
    let ok = h
        .post_with_cookie(
            "/api/databases/ledger_prod/extensions",
            r#"{"name":"pgvector"}"#,
            &cookie,
        )
        .await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT, "{}", ok.json);
    assert!(
        h.platform
            .calls()
            .contains(&format!("install_packages postgresql-{MAJOR}-pgvector"))
    );
    let bad = h
        .post_with_cookie(
            "/api/databases/ledger_prod/extensions",
            r#"{"name":"postgis"}"#,
            &cookie,
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let shown = h
        .get_with_cookie("/api/databases/ledger_prod", &cookie)
        .await;
    assert_eq!(shown.json["extensions"], serde_json::json!(["pgvector"]));
}

#[tokio::test]
async fn a_read_only_token_sees_databases_and_changes_nothing() {
    let (h, _cookie) = signed_in().await;
    h.platform.set_postgres_major(MAJOR);
    let token = h.machine_token(true).await;
    let listed = h.get_with_bearer("/api/databases", &token).await;
    assert_eq!(listed.status, StatusCode::OK);
    let status = h.get_with_bearer("/api/postgres", &token).await;
    assert_eq!(status.status, StatusCode::OK);
    let refused = h
        .post_with_bearer("/api/databases", r#"{"name":"ledger_prod"}"#, &token)
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    let install = h
        .post_with_bearer("/api/postgres/install", "", &token)
        .await;
    assert_eq!(install.status, StatusCode::FORBIDDEN);
    assert!(h.platform.sql().is_empty());
}
