mod support;

use axum::http::StatusCode;
use ferrum_core::github::Api;
use support::github_stub::{INSTALLATION_ID, ORG_APP_ID, ORG_INSTALLATION_ID, ORG_REPO};
use support::{HOSTNAME, connected_to_stub, harness, signed_in, signed_in_and_connected};

const ME: &str = "irixsoft";

#[tokio::test]
async fn the_manifest_names_this_host_and_asks_for_read_only_access() {
    let (h, cookie) = signed_in().await;
    let res = h.post_with_cookie("/api/github/connect", "", &cookie).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);

    let m = &res.json["manifest"];
    assert_eq!(
        m["hook_attributes"]["url"],
        format!("https://{HOSTNAME}/api/github/webhook")
    );
    assert_eq!(
        m["redirect_url"],
        format!("https://{HOSTNAME}/api/github/callback")
    );
    assert_eq!(m["default_permissions"]["contents"], "read");
    assert_eq!(m["public"], false);
    assert_eq!(m["default_events"], serde_json::json!(["push"]));
    assert!(
        m["default_permissions"]
            .as_object()
            .unwrap()
            .values()
            .all(|v| v == "read"),
        "ferrum never writes to a repository: {m}"
    );
}

#[tokio::test]
async fn the_handoff_carries_the_state_in_the_form_action() {
    let (h, cookie) = signed_in().await;
    let res = h.post_with_cookie("/api/github/connect", "", &cookie).await;

    let state = res.json["state"].as_str().unwrap();
    assert!(!state.is_empty());
    assert_eq!(
        res.json["action"],
        format!("https://github.com/settings/apps/new?state={state}")
    );
}

#[tokio::test]
async fn connecting_requires_authentication_but_the_callback_does_not() {
    let h = harness().await;
    assert_eq!(
        h.post("/api/github/connect", "").await.status,
        StatusCode::UNAUTHORIZED
    );

    let res = h.get("/api/github/callback?code=x&state=unknown").await;
    assert_ne!(
        res.status,
        StatusCode::UNAUTHORIZED,
        "a SameSite=Strict cookie is never sent on a redirect from github.com"
    );
}

const FAILED: &str = "/settings?github=failed&reason=";

fn landed_on(res: &support::Res) -> &str {
    assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.text);
    res.header("location")
        .expect("a redirect names where to go")
}

#[tokio::test]
async fn an_unknown_state_is_refused_before_the_code_is_exchanged() {
    let h = harness().await;
    let res = h
        .get("/api/github/callback?code=realcode&state=never-issued")
        .await;
    assert_eq!(
        landed_on(&res),
        format!(
            "{FAILED}That%20connection%20attempt%20expired.%20Start%20again%20from%20Settings."
        )
    );
}

#[tokio::test]
async fn a_callback_without_a_code_does_not_consume_the_state() {
    let (h, cookie) = signed_in().await;
    let state = h.connect_state(&cookie).await;

    let missing = h.get(&format!("/api/github/callback?state={state}")).await;
    assert_eq!(
        landed_on(&missing),
        format!("{FAILED}GitHub%20did%20not%20send%20a%20code.")
    );
    let res = h
        .get(&format!("/api/github/callback?code=x&state={state}"))
        .await;
    assert!(
        !landed_on(&res).contains("expired"),
        "a missing code must not burn the state: {}",
        res.text
    );
}

#[tokio::test]
async fn a_state_can_only_be_used_once() {
    let (h, cookie) = signed_in().await;
    let state = h.connect_state(&cookie).await;

    let first = h
        .get(&format!("/api/github/callback?code=x&state={state}"))
        .await;
    let second = h
        .get(&format!("/api/github/callback?code=x&state={state}"))
        .await;

    assert_eq!(
        landed_on(&first),
        format!("{FAILED}GitHub%20refused%20the%20connection.%20Start%20again%20from%20Settings."),
        "the stub answers no conversion, and that is GitHub refusing"
    );
    assert!(landed_on(&second).contains("expired"));
}

#[tokio::test]
async fn only_a_refusal_the_user_can_act_on_lands_on_the_card() {
    let (h, cookie) = signed_in().await;
    let state = h.connect_state(&cookie).await;
    sqlx::query("DROP TABLE github_state")
        .execute(&h.db.pool)
        .await
        .unwrap();

    let res = h
        .get(&format!("/api/github/callback?code=x&state={state}"))
        .await;
    assert_eq!(res.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(res.json["error"].is_string(), "{}", res.text);
}

#[tokio::test]
async fn an_organisation_is_registered_under_its_own_url_with_its_own_name() {
    let (h, cookie) = signed_in().await;
    let res = h
        .post_with_cookie("/api/github/connect", r#"{"organization":"acme"}"#, &cookie)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let state = res.json["state"].as_str().unwrap();
    assert_eq!(
        res.json["action"],
        format!("https://github.com/organizations/acme/settings/apps/new?state={state}")
    );
    assert_eq!(
        res.json["manifest"]["name"],
        "ferrum-acme-panel-example-com"
    );
    assert_eq!(res.json["manifest"]["public"], false);

    let bad = h
        .post_with_cookie(
            "/api/github/connect",
            r#"{"organization":"acme corp"}"#,
            &cookie,
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.json);
}

#[tokio::test]
async fn status_is_honest_before_anything_is_connected() {
    let (h, cookie) = signed_in().await;
    let res = h.get_with_cookie("/api/github/status", &cookie).await;
    assert_eq!(res.json["connected"], false);
    assert_eq!(res.json["connections"], serde_json::json!([]));
}

#[tokio::test]
async fn status_lists_every_connection_without_leaking_its_secrets() {
    let (h, cookie) = signed_in().await;
    h.connect_github().await;
    h.connect_org_github().await;

    let res = h.get_with_cookie("/api/github/status", &cookie).await;
    assert_eq!(res.json["connected"], true);
    let connections = res.json["connections"].as_array().unwrap();
    assert_eq!(connections.len(), 2);
    assert_eq!(connections[0]["app_name"], "ferrum-panel-example");
    assert_eq!(connections[0]["account"], ME);
    assert_eq!(connections[0]["account_type"], "user");
    assert_eq!(connections[1]["account"], "acme");
    assert_eq!(connections[1]["account_type"], "organization");
    assert!(!res.text.contains("PRIVATE KEY"), "{}", res.text);
    assert!(!res.text.contains("whsec"), "{}", res.text);
}

#[tokio::test]
async fn disconnecting_one_app_keeps_the_other() {
    let (h, cookie) = signed_in().await;
    h.connect_github().await;
    h.connect_org_github().await;

    assert_eq!(
        h.delete_with_cookie("/api/github/12345", &cookie)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.delete_with_cookie("/api/github/12345", &cookie)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let res = h.get_with_cookie("/api/github/status", &cookie).await;
    assert_eq!(res.json["connected"], true);
    assert_eq!(res.json["connections"][0]["account"], "acme");

    assert_eq!(
        h.delete_with_cookie(&format!("/api/github/{ORG_APP_ID}"), &cookie)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.get_with_cookie("/api/github/status", &cookie).await.json["connected"],
        false
    );
}

#[tokio::test]
async fn a_read_only_token_cannot_connect_or_disconnect() {
    let h = harness().await;
    let token = h.machine_token(true).await;
    h.connect_github().await;

    assert_eq!(
        h.post_with_bearer("/api/github/connect", "", &token)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.delete_with_bearer("/api/github/12345", &token)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        h.get_with_bearer("/api/github/status", &token).await.json["connected"],
        true,
        "a read-only token still reads"
    );
}

#[tokio::test]
async fn the_installation_is_discovered_once_and_remembered() {
    let (h, github) = connected_to_stub().await;

    assert_eq!(
        h.github_api.installation_id(&h.db, ME).await.unwrap(),
        INSTALLATION_ID
    );
    assert_eq!(
        ferrum_core::github::by_account(&h.db, ME)
            .await
            .unwrap()
            .unwrap()
            .installation_id,
        Some(INSTALLATION_ID),
        "the discovered installation must be written down"
    );
    assert_eq!(
        h.github_api
            .installation_id(&h.db, "IrixSoft")
            .await
            .unwrap(),
        INSTALLATION_ID,
        "logins are case-insensitive"
    );
    assert_eq!(github.mint_calls(), 0, "discovery must not mint a token");
}

#[tokio::test]
async fn an_uninstalled_app_says_so_rather_than_failing_obscurely() {
    let (h, github) = connected_to_stub().await;
    github.uninstall();

    let e = h.github_api.installation_id(&h.db, ME).await.unwrap_err();
    assert!(format!("{e}").contains("not installed"), "{e}");
}

#[tokio::test]
async fn each_account_has_its_own_installation_token_and_repositories() {
    let (h, github) = connected_to_stub().await;
    h.connect_org_github().await;

    let mine = h.github_api.installation_token(&h.db, ME).await.unwrap();
    let theirs = h
        .github_api
        .installation_token(&h.db, "acme")
        .await
        .unwrap();
    assert!(mine.contains(&format!("_{INSTALLATION_ID}_")), "{mine}");
    assert!(
        theirs.contains(&format!("_{ORG_INSTALLATION_ID}_")),
        "{theirs}"
    );
    assert_eq!(github.mint_calls(), 2);
    assert_eq!(
        ferrum_core::github::by_account(&h.db, "acme")
            .await
            .unwrap()
            .unwrap()
            .installation_id,
        Some(ORG_INSTALLATION_ID)
    );

    let e = h
        .github_api
        .installation_token(&h.db, "someone")
        .await
        .unwrap_err();
    assert!(
        format!("{e}").contains("No GitHub App is connected for someone"),
        "{e}"
    );

    let repos = h.github_api.repos(&h.db).await.unwrap();
    let names: Vec<&str> = repos.iter().map(|r| r.full_name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            ORG_REPO,
            "irixsoft/ledger",
            "irixsoft/notes",
            "irixsoft/panel"
        ]
    );
}

#[tokio::test]
async fn a_cached_token_is_reused_and_a_stale_one_is_replaced() {
    let (h, github) = connected_to_stub().await;

    let first = h.github_api.installation_token(&h.db, ME).await.unwrap();
    let second = h.github_api.installation_token(&h.db, ME).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(
        github.mint_calls(),
        1,
        "a valid token must not be re-minted"
    );

    github.tokens_expire_in(-1);
    h.github_api.forget();
    let third = h.github_api.installation_token(&h.db, ME).await.unwrap();
    assert_ne!(third, first);
    assert_eq!(github.mint_calls(), 2);
}

#[tokio::test]
async fn a_token_about_to_expire_is_refreshed_early() {
    let (h, github) = connected_to_stub().await;

    github.tokens_expire_in(30);
    let first = h.github_api.installation_token(&h.db, ME).await.unwrap();
    let second = h.github_api.installation_token(&h.db, ME).await.unwrap();

    assert_ne!(first, second);
    assert_eq!(
        github.mint_calls(),
        2,
        "a token expiring mid-clone is a deploy that fails halfway"
    );
}

#[tokio::test]
async fn minting_a_token_without_a_connection_says_to_connect() {
    let h = harness().await;
    let e = h
        .github_api
        .installation_token(&h.db, ME)
        .await
        .unwrap_err();
    assert!(format!("{e}").contains("not connected"), "{e}");
}

#[tokio::test]
async fn a_fresh_api_does_not_inherit_another_ones_token() {
    let (h, github) = connected_to_stub().await;
    h.github_api.installation_token(&h.db, ME).await.unwrap();

    let other = Api::at(&github.base);
    other.installation_token(&h.db, ME).await.unwrap();
    assert_eq!(
        github.mint_calls(),
        2,
        "the cache belongs to the instance, not to the process"
    );
}

#[tokio::test]
async fn repositories_come_back_for_the_installation_only() {
    let (h, cookie, github) = signed_in_and_connected().await;

    let res = h.get_with_cookie("/api/github/repos", &cookie).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);

    let repos = res.json.as_array().unwrap();
    assert_eq!(repos.len(), 3, "both pages must be followed: {}", res.json);
    assert_eq!(repos[0]["full_name"], "irixsoft/ledger");
    assert_eq!(repos[0]["default_branch"], "main");
    assert_eq!(repos[0]["private"], false);
    assert_eq!(repos[2]["full_name"], "irixsoft/panel");
    assert_eq!(
        github.repo_page_calls(),
        2,
        "a listing that stops at page one hides repositories with no error"
    );
}

#[tokio::test]
async fn tags_come_back_newest_first_and_an_untagged_repository_is_empty() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    let res = h
        .get_with_cookie("/api/github/repos/irixsoft/ledger/tags", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let names: Vec<&str> = res
        .json
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["v1.10.0", "v1.9.0", "v1.4.0"]);
    assert_eq!(res.json[0]["sha"].as_str().unwrap().len(), 40);

    let none = h
        .get_with_cookie("/api/github/repos/irixsoft/untagged/tags", &cookie)
        .await;
    assert_eq!(none.status, StatusCode::OK);
    assert_eq!(none.json, serde_json::json!([]));
}

#[tokio::test]
async fn listing_repositories_without_a_connection_says_so() {
    let (h, cookie) = signed_in().await;
    let res = h.get_with_cookie("/api/github/repos", &cookie).await;

    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        res.json["error"].as_str().unwrap().contains("GitHub"),
        "{}",
        res.json
    );
}

#[tokio::test]
async fn listing_repositories_before_the_app_is_installed_says_so() {
    let (h, cookie, github) = signed_in_and_connected().await;
    github.uninstall();

    let res = h.get_with_cookie("/api/github/repos", &cookie).await;
    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        res.json["error"]
            .as_str()
            .unwrap()
            .contains("not installed"),
        "{}",
        res.json
    );
}

#[tokio::test]
async fn listing_repositories_requires_authentication() {
    let h = harness().await;
    assert_eq!(
        h.get("/api/github/repos").await.status,
        StatusCode::UNAUTHORIZED
    );
}
