mod support;

use axum::http::StatusCode;
use ferrum_platform::{Exit, Platform};
use std::path::Path;
use std::time::Duration;
use support::signed_in;

fn deployed(h: &support::Harness) {
    h.platform
        .write_file(
            Path::new("/var/lib/ferrum/apps/ledger/current/.git/HEAD"),
            "ref: refs/heads/main\n",
            0o644,
        )
        .unwrap();
}

async fn settled(h: &support::Harness, cookie: &str, id: &str) -> serde_json::Value {
    for _ in 0..200 {
        let runs = h.get_with_cookie("/api/apps/ledger/commands", cookie).await;
        if let Some(run) = runs
            .json
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id && !r["finished_at"].is_null())
        {
            return run.clone();
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the run never finished");
}

#[tokio::test]
async fn a_command_runs_in_the_current_release_and_its_log_streams() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    deployed(&h);
    h.platform
        .script_run("seed:admin", &["seeding", "done"], Exit::Code(0));
    let res = h
        .post_with_cookie(
            "/api/apps/ledger/commands",
            r#"{"command":"bun run seed:admin 'pw'"}"#,
            &cookie,
        )
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.json);
    let id = res.json["id"].as_str().unwrap().to_string();
    assert_eq!(res.json["command"], "bun run seed:admin 'pw'");

    let run = settled(&h, &cookie, &id).await;
    assert_eq!(run["exit"], "ok");
    let spec = &h.platform.runs()[0];
    assert_eq!(spec.user, "ferrum-ledger");
    assert_eq!(spec.cwd, Path::new("/var/lib/ferrum/apps/ledger/current"));
    assert!(spec.unit.starts_with("ferrum-run-ledger-"));

    let log = h
        .stream_with_cookie(&format!("/api/commands/{id}/log"), &cookie)
        .await;
    assert_eq!(log.status, StatusCode::OK);
    assert!(
        log.header("content-type")
            .unwrap()
            .starts_with("text/event-stream")
    );
    assert!(log.text.contains("$ bun run seed:admin"), "{}", log.text);
    assert!(log.text.contains("seeding"), "{}", log.text);
    assert!(log.text.contains(r#"event: done"#), "{}", log.text);
    assert!(log.text.contains(r#"{"exit":"ok"}"#), "{}", log.text);
}

#[tokio::test]
async fn a_command_is_refused_before_the_first_deploy_and_beside_a_running_one() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    let res = h
        .post_with_cookie("/api/apps/ledger/commands", r#"{"command":"ls"}"#, &cookie)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.json);
    assert!(res.json["error"].as_str().unwrap().contains("Deploy"));

    deployed(&h);
    let blank = h
        .post_with_cookie("/api/apps/ledger/commands", r#"{"command":"  "}"#, &cookie)
        .await;
    assert_eq!(blank.status, StatusCode::BAD_REQUEST, "{}", blank.json);

    let gate = h.platform.gate("wait");
    let first = h
        .post_with_cookie(
            "/api/apps/ledger/commands",
            r#"{"command":"wait"}"#,
            &cookie,
        )
        .await;
    assert_eq!(first.status, StatusCode::ACCEPTED, "{}", first.json);
    let second = h
        .post_with_cookie("/api/apps/ledger/commands", r#"{"command":"ls"}"#, &cookie)
        .await;
    assert_eq!(second.status, StatusCode::CONFLICT, "{}", second.json);
    gate.open();
    let done = settled(&h, &cookie, first.json["id"].as_str().unwrap()).await;
    assert_eq!(done["exit"], "ok");

    let missing = h
        .stream_with_cookie("/api/commands/nope/log", &cookie)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}
