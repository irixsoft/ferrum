mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ferrum_platform::{Exit, Platform};
use support::{StubHealth, signed_in, signed_in_and_connected, static_app_json};
use tokio_stream::StreamExt;
use tower::ServiceExt;

#[tokio::test]
async fn the_tail_comes_from_journald_or_nginx_with_three_levels() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    h.platform.journal(
        "ferrum-app-ledger",
        &[
            (6, "Listening on 127.0.0.1:20000"),
            (4, "slow query"),
            (3, "upstream returned 503"),
        ],
    );
    h.platform
        .write_file(
            std::path::Path::new("/var/log/nginx/ferrum-ledger.access.log"),
            "203.0.113.44 - - [02/Sep/2026:12:05:14 +0000] \"GET /api HTTP/2.0\" 200 12 \"-\" \"curl\"\n",
            0o644,
        )
        .unwrap();

    let all = h.get_with_cookie("/api/apps/ledger/logs", &cookie).await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.json);
    let lines = all.json.as_array().unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["level"], "info");
    assert_eq!(lines[1]["level"], "warn");
    assert_eq!(lines[2]["level"], "error");
    assert_eq!(lines[2]["text"], "upstream returned 503");
    assert!(lines[0]["at"].as_str().unwrap().ends_with('Z'));

    let two = h
        .get_with_cookie("/api/apps/ledger/logs?source=app&lines=2", &cookie)
        .await;
    assert_eq!(two.json.as_array().unwrap().len(), 2);
    assert_eq!(two.json[0]["text"], "slow query");

    let access = h
        .get_with_cookie("/api/apps/ledger/logs?source=access", &cookie)
        .await;
    assert_eq!(access.json[0]["at"], "2026-09-02T12:05:14Z");
    assert_eq!(
        access.json[0]["text"],
        "203.0.113.44 - - \"GET /api HTTP/2.0\" 200 12 \"-\" \"curl\""
    );
    let errors = h
        .get_with_cookie("/api/apps/ledger/logs?source=error", &cookie)
        .await;
    assert!(errors.json.as_array().unwrap().is_empty());

    let bad = h
        .get_with_cookie("/api/apps/ledger/logs?source=build", &cookie)
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let not_followable = h
        .get_with_cookie("/api/apps/ledger/logs?source=access&follow=1", &cookie)
        .await;
    assert_eq!(not_followable.status, StatusCode::BAD_REQUEST);
    let missing = h.get_with_cookie("/api/apps/nope/logs", &cookie).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let token = h.machine_token(true).await;
    assert_eq!(
        h.get_with_bearer("/api/apps/ledger/logs", &token)
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_follow_streams_live_lines_and_ends_journalctl_when_the_client_leaves() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    h.platform.journal("ferrum-app-ledger", &[(6, "first")]);
    let req = Request::builder()
        .uri("/api/apps/ledger/logs?follow=1&lines=1")
        .header(header::ACCEPT, "text/event-stream")
        .header(header::COOKIE, format!("ferrum_session={cookie}"))
        .header(header::USER_AGENT, support::USER_AGENT)
        .body(Body::empty())
        .unwrap();
    let res = h.app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        res.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let mut body = res.into_body().into_data_stream();
    let mut text = String::new();
    let wait = std::time::Duration::from_secs(5);
    while !text.contains("first") {
        let chunk = tokio::time::timeout(wait, body.next())
            .await
            .expect("the first line arrives")
            .unwrap()
            .unwrap();
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(text.contains("event: line"), "{text}");
    h.platform.journal("ferrum-app-ledger", &[(3, "later")]);
    while !text.contains("later") {
        let chunk = tokio::time::timeout(wait, body.next())
            .await
            .expect("the live line arrives")
            .unwrap()
            .unwrap();
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
    assert!(text.contains("\"level\":\"error\""), "{text}");
    assert!(!text.contains("event: done"));
    drop(body);
    for _ in 0..300 {
        if h.platform.follows_ended() == 1 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("journalctl kept running after the client left");
}

#[tokio::test]
async fn a_restart_is_refused_for_static_sites_undeployed_apps_and_during_a_deploy() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.create_app_from(&static_app_json("docs"), &cookie).await;
    let docs = h
        .post_with_cookie("/api/apps/docs/restart", "", &cookie)
        .await;
    assert_eq!(docs.status, StatusCode::BAD_REQUEST, "{}", docs.json);

    h.create_app("ledger", &cookie).await;
    let fresh = h
        .post_with_cookie("/api/apps/ledger/restart", "", &cookie)
        .await;
    assert_eq!(fresh.status, StatusCode::CONFLICT, "{}", fresh.json);

    let health = StubHealth::start(200).await;
    h.force_port("ledger", health.port).await;
    h.platform
        .script_run("bun run build", &["Compiled"], Exit::Code(0));
    h.platform.set_active("ferrum-app-ledger");
    let gate = h.platform.gate("bun run build");
    let queued = h
        .post_with_cookie("/api/apps/ledger/deploys", "", &cookie)
        .await;
    assert_eq!(queued.status, StatusCode::ACCEPTED, "{}", queued.json);
    let id = queued.json["id"].as_str().unwrap().to_string();
    for _ in 0..600 {
        let d = h
            .get_with_cookie(&format!("/api/deploys/{id}"), &cookie)
            .await;
        if d.json["state"] == "Building" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let busy = h
        .post_with_cookie("/api/apps/ledger/restart", "", &cookie)
        .await;
    assert_eq!(busy.status, StatusCode::CONFLICT, "{}", busy.json);
    gate.open();
    assert_eq!(h.wait_for_deploy(&id, &cookie).await["outcome"], "Live");

    let before = h.platform.calls_matching("service restart").len();
    let ok = h
        .post_with_cookie("/api/apps/ledger/restart", "", &cookie)
        .await;
    assert_eq!(ok.status, StatusCode::ACCEPTED, "{}", ok.json);
    let restarts = h
        .platform
        .calls_matching("service restart ferrum-app-ledger");
    assert_eq!(restarts.len(), before + 1, "{restarts:?}");

    let token = h.machine_token(true).await;
    let read_only = h
        .post_with_bearer("/api/apps/ledger/restart", "", &token)
        .await;
    assert_eq!(read_only.status, StatusCode::FORBIDDEN);
    let missing = h
        .post_with_cookie("/api/apps/nope/restart", "", &cookie)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}
