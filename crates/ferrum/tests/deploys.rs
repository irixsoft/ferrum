mod support;

use axum::http::StatusCode;
use ferrum_core::github::webhook;
use ferrum_platform::Exit;
use serde_json::Value;
use support::github_stub::{HEAD_MESSAGE, HEAD_SHA, LATEST_TAG};
use support::{
    Harness, StubHealth, WEBHOOK_SECRET, push_payload, signed_in, signed_in_and_connected,
    static_app_json,
};

async fn wait_for_state(h: &Harness, id: &str, cookie: &str, state: &str) -> Value {
    for _ in 0..600 {
        let res = h
            .get_with_cookie(&format!("/api/deploys/{id}"), cookie)
            .await;
        if res.json["state"] == state {
            return res.json;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("deploy {id} never reached {state}");
}

#[tokio::test]
async fn a_manual_deploy_resolves_the_branch_head_runs_and_streams_its_log() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.create_app("ledger", &cookie).await;
    let health = StubHealth::start(200).await;
    h.force_port("ledger", health.port).await;
    h.platform
        .script_run("bun run build", &["Compiled"], Exit::Code(0));
    h.platform.set_active("ferrum-app-ledger");

    let res = h
        .post_with_cookie("/api/apps/ledger/deploys", "", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.json);
    assert_eq!(res.json["commit_sha"], HEAD_SHA);
    assert_eq!(res.json["commit_message"], HEAD_MESSAGE);
    assert_eq!(res.json["author"], "saeed");
    assert_eq!(res.json["trigger"], "manual");
    assert_eq!(res.json["git_ref"], "main");
    assert_eq!(res.json["app_slug"], "ledger");
    let id = res.json["id"].as_str().unwrap().to_string();

    let done = h.wait_for_deploy(&id, &cookie).await;
    assert_eq!(done["outcome"], "Live", "{done}");
    assert!(done["state"].is_null());
    assert_eq!(done["steps"].as_array().unwrap().len(), 12);
    assert_eq!(done["steps"][4]["status"], "done");
    assert!(done["duration_secs"].is_number());
    assert!(health.hits() >= 1);

    let log = h
        .stream_with_cookie(&format!("/api/deploys/{id}/log"), &cookie)
        .await;
    assert_eq!(log.status, StatusCode::OK);
    assert!(
        log.header("content-type")
            .unwrap()
            .starts_with("text/event-stream"),
        "{:?}",
        log.headers
    );
    assert!(log.text.contains("→ Building"), "{}", log.text);
    assert!(log.text.contains("Compiled"), "{}", log.text);
    assert!(log.text.contains("event: done"), "{}", log.text);
    assert!(log.text.contains("\"outcome\":\"Live\""), "{}", log.text);
    assert!(
        !log.text.contains("ghs_"),
        "the installation token must never reach the log"
    );

    let app = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert_eq!(app.json["deployed"], true);
    assert_eq!(app.json["status"], "live");
    assert_eq!(app.json["current_release"]["commit_sha"], HEAD_SHA);
    assert_eq!(app.json["current_release"]["current"], true);
    let listed = h.get_with_cookie("/api/apps", &cookie).await;
    assert_eq!(listed.json[0]["status"], "live");

    let releases = h
        .get_with_cookie("/api/apps/ledger/releases", &cookie)
        .await;
    assert_eq!(releases.json.as_array().unwrap().len(), 1);
    assert_eq!(releases.json[0]["current"], true);
    let all = h.get_with_cookie("/api/deploys", &cookie).await;
    assert_eq!(all.json[0]["id"], id);
    let running = h.get_with_cookie("/api/deploys?running=1", &cookie).await;
    assert!(running.json.is_null());
}

#[tokio::test]
async fn release_tracking_deploys_the_latest_release_tag() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    let json =
        static_app_json("docs").replace("\"tracking\":\"branch\"", "\"tracking\":\"releases\"");
    assert!(json.contains("releases"));
    h.create_app_from(&json, &cookie).await;
    let res = h
        .post_with_cookie("/api/apps/docs/deploys", "{}", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.json);
    assert_eq!(res.json["git_ref"], LATEST_TAG);
    assert!(res.json["commit_sha"].as_str().unwrap().starts_with("140"));
    let id = res.json["id"].as_str().unwrap();
    assert_eq!(h.wait_for_deploy(id, &cookie).await["outcome"], "Live");
    assert!(
        h.platform
            .calls()
            .iter()
            .any(|c| c.starts_with("git_clone https://github.com/irixsoft/ledger.git v1.4.0 ")),
        "{:#?}",
        h.platform.calls()
    );
    let explicit = h
        .post_with_cookie("/api/apps/docs/deploys", r#"{"ref":"v1.3.0"}"#, &cookie)
        .await;
    assert_eq!(explicit.status, StatusCode::ACCEPTED, "{}", explicit.json);
    assert_eq!(explicit.json["git_ref"], "v1.3.0");
    let missing = h
        .post_with_cookie("/api/apps/docs/deploys", r#"{"ref":"missing"}"#, &cookie)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.json);
}

#[tokio::test]
async fn a_verified_push_on_the_tracked_branch_enqueues_a_deploy() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.create_app_from(&static_app_json("ledger"), &cookie).await;
    let body = push_payload("irixsoft/ledger", "refs/heads/main", "a3f9c2d4e81b06f5c9a2");
    let sig = webhook::sign(WEBHOOK_SECRET, &body);
    let res = h.webhook("push", "d-1", &sig, &body).await;
    assert_eq!(res.status, StatusCode::NO_CONTENT);
    let deploys = h.get_with_cookie("/api/apps/ledger/deploys", &cookie).await;
    assert_eq!(deploys.json[0]["trigger"], "webhook");
    assert_eq!(deploys.json[0]["commit_sha"], "a3f9c2d4e81b06f5c9a2");
    let id = deploys.json[0]["id"].as_str().unwrap();
    assert_eq!(h.wait_for_deploy(id, &cookie).await["outcome"], "Live");

    let other = push_payload("irixsoft/ledger", "refs/heads/dev", "ffff");
    h.webhook(
        "push",
        "d-2",
        &webhook::sign(WEBHOOK_SECRET, &other),
        &other,
    )
    .await;
    let again = h.webhook("push", "d-1", &sig, &body).await;
    assert_eq!(
        again.status,
        StatusCode::NO_CONTENT,
        "a redelivery is acknowledged"
    );
    assert_eq!(
        h.get_with_cookie("/api/apps/ledger/deploys", &cookie)
            .await
            .json
            .as_array()
            .unwrap()
            .len(),
        1,
        "neither the other branch nor the redelivery deploys"
    );
}

#[tokio::test]
async fn rollback_repoints_without_a_build_and_offers_the_snapshot_choice() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.platform.set_postgres_major(18);
    let json = static_app_json("docs").replace(
        "\"build\":\"bun run build\"",
        "\"build\":\"bun run build\",\"migrate\":\"bun run db:migrate\"",
    );
    h.create_app_from(&json, &cookie).await;
    let db = h
        .post_with_cookie(
            "/api/databases",
            r#"{"name":"docs_prod","app_slug":"docs"}"#,
            &cookie,
        )
        .await;
    assert_eq!(db.status, StatusCode::CREATED, "{}", db.json);

    let first = h
        .post_with_cookie("/api/apps/docs/deploys", r#"{"ref":"v1.0.0"}"#, &cookie)
        .await;
    let first_id = first.json["id"].as_str().unwrap().to_string();
    assert_eq!(
        h.wait_for_deploy(&first_id, &cookie).await["outcome"],
        "Live"
    );
    let second = h
        .post_with_cookie("/api/apps/docs/deploys", r#"{"ref":"v2.0.0"}"#, &cookie)
        .await;
    let second_id = second.json["id"].as_str().unwrap().to_string();
    let second = h.wait_for_deploy(&second_id, &cookie).await;
    assert_eq!(second["outcome"], "Live", "{second}");
    assert_eq!(second["snapshots"].as_array().unwrap().len(), 1);
    let snapshot_id = second["snapshots"][0]["id"].as_str().unwrap().to_string();

    let releases = h.get_with_cookie("/api/apps/docs/releases", &cookie).await;
    let list = releases.json.as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["current"], true, "newest first");
    let first_release = list[1]["id"].as_str().unwrap().to_string();

    let before = h.platform.calls().len();
    let res = h
        .post_with_cookie(
            "/api/apps/docs/rollback",
            &format!(r#"{{"release_id":"{first_release}","restore_deploy_id":"{second_id}"}}"#),
            &cookie,
        )
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.json);
    assert_eq!(res.json["trigger"], "rollback");
    let done = h
        .wait_for_deploy(res.json["id"].as_str().unwrap(), &cookie)
        .await;
    assert_eq!(done["outcome"], "Live", "{done}");
    let calls: Vec<String> = h.platform.calls().into_iter().skip(before).collect();
    assert!(
        !calls
            .iter()
            .any(|c| c.starts_with("run_scoped") || c.starts_with("git_clone")),
        "{calls:#?}"
    );
    let restore = calls
        .iter()
        .position(|c| c.starts_with("postgres_restore docs_prod"))
        .unwrap();
    let swap = calls
        .iter()
        .position(|c| c.starts_with("symlink_swap") && c.ends_with("/current"))
        .unwrap();
    assert!(restore < swap, "{calls:#?}");
    let app = h.get_with_cookie("/api/apps/docs", &cookie).await;
    assert_eq!(app.json["current_release"]["id"], first_release);

    let unknown = h
        .post_with_cookie(
            "/api/apps/docs/rollback",
            r#"{"release_id":"nope"}"#,
            &cookie,
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);

    let restored = h
        .post_with_cookie(
            &format!("/api/snapshots/{snapshot_id}/restore"),
            "",
            &cookie,
        )
        .await;
    assert_eq!(restored.status, StatusCode::NO_CONTENT, "{}", restored.json);
    assert_eq!(
        h.platform
            .calls_matching("postgres_restore docs_prod")
            .len(),
        2
    );
}

#[tokio::test]
async fn a_read_only_token_can_watch_but_not_deploy() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.create_app_from(&static_app_json("docs"), &cookie).await;
    let token = h.machine_token(true).await;
    for uri in [
        "/api/deploys",
        "/api/apps/docs/deploys",
        "/api/apps/docs/releases",
    ] {
        assert_eq!(
            h.get_with_bearer(uri, &token).await.status,
            StatusCode::OK,
            "{uri}"
        );
    }
    assert_eq!(
        h.post_with_bearer("/api/apps/docs/deploys", "{}", &token)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.post_with_bearer("/api/apps/docs/rollback", r#"{"release_id":"x"}"#, &token)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn cancelling_a_queued_deploy_removes_it_and_a_running_one_is_refused() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.create_app_from(&static_app_json("a"), &cookie).await;
    h.create_app_from(&static_app_json("b"), &cookie).await;
    let gate = h.platform.gate("bun run build");

    let first = h.post_with_cookie("/api/apps/a/deploys", "", &cookie).await;
    let first_id = first.json["id"].as_str().unwrap().to_string();
    wait_for_state(&h, &first_id, &cookie, "Building").await;
    let second = h.post_with_cookie("/api/apps/b/deploys", "", &cookie).await;
    assert_eq!(second.json["state"], "Queued");
    assert_eq!(second.json["queue_position"], 1);
    let second_id = second.json["id"].as_str().unwrap().to_string();

    let running = h.get_with_cookie("/api/deploys?running=1", &cookie).await;
    assert_eq!(running.json["id"], first_id);
    assert_eq!(
        h.get_with_cookie("/api/apps/a", &cookie).await.json["status"],
        "building"
    );

    let refused = h
        .post_with_cookie(&format!("/api/deploys/{first_id}/cancel"), "", &cookie)
        .await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    let cancelled = h
        .post_with_cookie(&format!("/api/deploys/{second_id}/cancel"), "", &cookie)
        .await;
    assert_eq!(cancelled.status, StatusCode::NO_CONTENT);
    assert_eq!(
        h.get_with_cookie(&format!("/api/deploys/{second_id}"), &cookie)
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    gate.open();
    assert_eq!(
        h.wait_for_deploy(&first_id, &cookie).await["outcome"],
        "Live"
    );
    assert!(
        !h.platform
            .calls()
            .iter()
            .any(|c| c.starts_with("git_clone") && c.contains("/apps/b/")),
        "the cancelled deploy must never run"
    );
}

#[tokio::test]
async fn a_deploy_needs_github_and_says_so() {
    let (h, cookie) = signed_in().await;
    h.create_app_from(&static_app_json("docs"), &cookie).await;
    let res = h
        .post_with_cookie("/api/apps/docs/deploys", "", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE, "{}", res.json);
    assert!(res.json["error"].as_str().unwrap().contains("GitHub"));
    assert_eq!(
        h.post_with_cookie("/api/apps/nope/deploys", "", &cookie)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}
