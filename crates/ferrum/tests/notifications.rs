mod support;

use axum::http::StatusCode;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ferrum_core::events::{self, Kind};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;
use support::signed_in;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// The P-256 base point: a valid browser key, though nobody here decrypts with it.
const P256DH: &str =
    "BGsX0fLhLEJH-Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT-NC4v4af5uO5-tKfA-eFivOM1drMV7Oy7ZAaDe_UfU";

struct Received {
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

/// A push service on loopback: reports each request and answers with `status`.
async fn push_service(status: Arc<AtomicU16>) -> (String, mpsc::UnboundedReceiver<Received>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let tx = tx.clone();
            let status = status.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let headers: HashMap<String, String> = head
                    .lines()
                    .skip(1)
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                    .collect();
                let length: usize = headers
                    .get("content-length")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
                while buf.len() < head_end + length {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body = buf[head_end..].to_vec();
                let _ = tx.send(Received { headers, body });
                let code = status.load(Ordering::SeqCst);
                let reply =
                    format!("HTTP/1.1 {code} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    (base, rx)
}

fn subscription(endpoint: &str) -> String {
    serde_json::json!({
        "endpoint": endpoint,
        "expirationTime": null,
        "keys": { "p256dh": P256DH, "auth": URL_SAFE_NO_PAD.encode([7u8; 16]) },
    })
    .to_string()
}

async fn next(rx: &mut mpsc::UnboundedReceiver<Received>) -> Received {
    tokio::time::timeout(Duration::from_secs(20), rx.recv())
        .await
        .expect("the push service was reached")
        .unwrap()
}

#[tokio::test]
async fn an_event_reaches_a_registered_device_and_a_gone_subscription_is_dropped() {
    let (h, cookie) = signed_in().await;
    let status = Arc::new(AtomicU16::new(201));
    let (base, mut rx) = push_service(status.clone()).await;
    let endpoint = format!("{base}/push/abc");

    let key = h.get_with_cookie("/api/push/vapid", &cookie).await;
    assert_eq!(key.status, StatusCode::OK, "{}", key.json);
    let key = key.json["key"].as_str().unwrap().to_string();

    let registered = h
        .post_with_cookie("/api/push/devices", &subscription(&endpoint), &cookie)
        .await;
    assert_eq!(
        registered.status,
        StatusCode::CREATED,
        "{}",
        registered.json
    );
    assert!(registered.json.get("endpoint").is_none());
    assert!(registered.json.get("auth").is_none());

    let mut db = h.db.clone();
    db.events_tx = Some(ferrum_core::push::spawn_fanout(
        h.db.clone(),
        ferrum_core::http::client(),
    ));
    events::emit(
        &db,
        Kind::DeployLive,
        None,
        "ledger",
        "ledger is live at abc1234.",
        Some("/apps/ledger"),
    )
    .await
    .unwrap();

    let got = next(&mut rx).await;
    let authorization = &got.headers["authorization"];
    let (token, k) = authorization
        .strip_prefix("vapid t=")
        .and_then(|rest| rest.split_once(", k="))
        .expect("a VAPID authorization");
    assert_eq!(k, key);
    assert_eq!(token.split('.').count(), 3);
    assert_eq!(got.headers["content-encoding"], "aes128gcm");
    assert_eq!(got.headers["ttl"], "86400");
    assert_eq!(&got.body[16..20], &4096u32.to_be_bytes());

    let mut stamped = false;
    for _ in 0..100 {
        let listed = h.get_with_cookie("/api/push/devices", &cookie).await;
        if !listed.json[0]["last_ok_at"].is_null() {
            stamped = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(stamped, "a delivery is stamped on the device");

    status.store(410, Ordering::SeqCst);
    let tried = h.post_with_cookie("/api/push/test", "", &cookie).await;
    assert_eq!(tried.status, StatusCode::OK, "{}", tried.json);
    assert_eq!(tried.json[0]["result"], "gone");
    next(&mut rx).await;
    let listed = h.get_with_cookie("/api/push/devices", &cookie).await;
    assert_eq!(listed.json, serde_json::json!([]));
}

#[tokio::test]
async fn prefs_default_on_change_one_at_a_time_and_refuse_an_unknown_kind() {
    let (h, cookie) = signed_in().await;
    let prefs = h.get_with_cookie("/api/push/prefs", &cookie).await;
    assert_eq!(
        prefs.json,
        serde_json::json!({"enabled": {"broke": true, "deploy_failed": true, "deploy_live": true, "update": true}})
    );

    let changed = h
        .put_with_cookie("/api/push/prefs", r#"{"deploy_live":false}"#, &cookie)
        .await;
    assert_eq!(changed.status, StatusCode::OK, "{}", changed.json);
    assert_eq!(changed.json["enabled"]["deploy_live"], false);
    assert_eq!(changed.json["enabled"]["update"], true);

    let unknown = h
        .put_with_cookie("/api/push/prefs", r#"{"role_kept":true}"#, &cookie)
        .await;
    assert_eq!(unknown.status, StatusCode::BAD_REQUEST);

    let bad = h
        .post_with_cookie(
            "/api/push/devices",
            r#"{"endpoint":"https://push.example.com/x","keys":{"p256dh":"AAAA","auth":"AAAA"}}"#,
            &cookie,
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.json);
}

#[tokio::test]
async fn the_events_log_lists_counts_and_marks_read_and_push_is_for_people_only() {
    let (h, cookie) = signed_in().await;
    let first = events::emit(&h.db, Kind::DeployFailed, None, "ledger", "failed", None)
        .await
        .unwrap();
    events::emit(&h.db, Kind::PortRemoved, None, "ledger", "route gone", None).await;

    let unread = h.get_with_cookie("/api/events/unread", &cookie).await;
    assert_eq!(unread.json["count"], 2);
    let listed = h
        .get_with_cookie("/api/events?unread=1&limit=50", &cookie)
        .await;
    assert_eq!(listed.json.as_array().unwrap().len(), 2);
    assert!(
        listed.json[0]["created_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );

    let read = h
        .post_with_cookie(
            "/api/events/read",
            &serde_json::json!({ "ids": [first.id] }).to_string(),
            &cookie,
        )
        .await;
    assert_eq!(read.status, StatusCode::NO_CONTENT);
    let unread = h.get_with_cookie("/api/events?unread=1", &cookie).await;
    assert_eq!(unread.json[0]["kind"], "port_removed");
    h.post_with_cookie("/api/events/read", r#"{"all":true}"#, &cookie)
        .await;
    let unread = h.get_with_cookie("/api/events/unread", &cookie).await;
    assert_eq!(unread.json["count"], 0);
    let all = h.get_with_cookie("/api/events", &cookie).await;
    assert_eq!(all.json.as_array().unwrap().len(), 2);

    let token = h.machine_token(false).await;
    let refused = h.get_with_bearer("/api/push/devices", &token).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    let allowed = h.get_with_bearer("/api/events/unread", &token).await;
    assert_eq!(allowed.status, StatusCode::OK);
}
