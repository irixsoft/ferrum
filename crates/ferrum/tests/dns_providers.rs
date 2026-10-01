mod support;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use support::signed_in;

const TOKEN: &str = "cf-token-that-must-not-leak";

type Log = Arc<Mutex<Vec<String>>>;

fn authorized(headers: &HeaderMap) -> bool {
    headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(&format!("Bearer {TOKEN}"))
}

async fn stub_cloudflare() -> (String, Log) {
    let log: Log = Arc::default();
    let app = Router::new()
        .route(
            "/zones",
            get(
                |State(log): State<Log>, headers: HeaderMap, Query(q): Query<HashMap<String, String>>| async move {
                    let name = q.get("name").cloned().unwrap_or_default();
                    log.lock().unwrap().push(format!("zones {name}"));
                    if !authorized(&headers) {
                        return (StatusCode::UNAUTHORIZED, Json(json!({"success": false})));
                    }
                    let result = if name == "example.com" {
                        json!([{ "id": "zone1", "name": "example.com" }])
                    } else {
                        json!([])
                    };
                    (StatusCode::OK, Json(json!({ "success": true, "result": result })))
                },
            ),
        )
        .route(
            "/zones/{zone}/dns_records",
            post(
                |State(log): State<Log>, Path(zone): Path<String>, Json(body): Json<Value>| async move {
                    log.lock().unwrap().push(format!("create {zone} {}", body["name"].as_str().unwrap_or_default()));
                    Json(json!({ "success": true, "result": { "id": "rec1" } }))
                },
            ),
        )
        .route(
            "/zones/{zone}/dns_records/{record}",
            delete(
                |State(log): State<Log>, Path((zone, record)): Path<(String, String)>| async move {
                    log.lock().unwrap().push(format!("delete {zone} {record}"));
                    Json(json!({ "success": true, "result": { "id": record } }))
                },
            ),
        )
        .with_state(log.clone());
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, log)
}

fn body(token: &str) -> String {
    json!({
        "name": "Cloudflare main",
        "kind": "cloudflare",
        "credentials": { "token": token },
        "zone": "example.com",
    })
    .to_string()
}

#[tokio::test]
async fn a_provider_is_proven_on_save_listed_without_its_token_and_removed() {
    let (h, cookie) = signed_in().await;
    let (base, log) = stub_cloudflare().await;
    h.db.set_setting("dns.cloudflare_api", &base).await.unwrap();

    let refused = h
        .post_with_cookie("/api/dns-providers", &body("wrong"), &cookie)
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.json);
    assert!(
        refused.json["error"]
            .as_str()
            .unwrap()
            .contains("refused the API token"),
        "{}",
        refused.json
    );

    let saved = h
        .post_with_cookie("/api/dns-providers", &body(TOKEN), &cookie)
        .await;
    assert_eq!(saved.status, StatusCode::CREATED, "{}", saved.json);
    assert!(!saved.text.contains(TOKEN));
    assert_eq!(saved.json["kind"], "cloudflare");
    let id = saved.json["id"].as_str().unwrap().to_string();
    assert!(
        log.lock().unwrap().ends_with(&[
            "zones _ferrum-probe.example.com".to_string(),
            "zones example.com".to_string(),
            "create zone1 _ferrum-probe.example.com".to_string(),
            "delete zone1 rec1".to_string(),
        ]),
        "{:?}",
        log.lock().unwrap()
    );

    let listed = h.get_with_cookie("/api/dns-providers", &cookie).await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.json.as_array().unwrap().len(), 1);
    assert_eq!(listed.json[0]["name"], "Cloudflare main");
    assert!(listed.json[0].get("credentials").is_none());
    assert!(!listed.text.contains(TOKEN));

    let gone = h
        .delete_with_cookie(&format!("/api/dns-providers/{id}"), &cookie)
        .await;
    assert_eq!(gone.status, StatusCode::NO_CONTENT);
    let again = h
        .delete_with_cookie(&format!("/api/dns-providers/{id}"), &cookie)
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_routes_sit_behind_login() {
    let (h, _) = signed_in().await;
    assert_eq!(
        h.get("/api/dns-providers").await.status,
        StatusCode::UNAUTHORIZED
    );
}
