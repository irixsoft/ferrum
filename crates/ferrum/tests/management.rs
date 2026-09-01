mod support;

use axum::http::StatusCode;
use support::*;

#[tokio::test]
async fn creating_a_user_returns_a_usable_enrollment_url() {
    let (h, cookie) = signed_in().await;
    let res = h
        .post_with_cookie("/api/users", r#"{"name":"Teammate"}"#, &cookie)
        .await;

    assert_eq!(res.status, StatusCode::OK);
    let url = res.json["enrollment_url"].as_str().unwrap().to_string();
    assert!(url.starts_with("https://"), "{url}");
    assert!(url.contains("/enroll/"), "{url}");

    let token = url.rsplit('/').next().unwrap();
    let mut key = soft_passkey();
    assert_eq!(
        h.register(&mut key, token).await.status,
        StatusCode::NO_CONTENT,
        "the printed link must actually enrol"
    );
}

#[tokio::test]
async fn creating_a_user_requires_authentication() {
    let h = harness().await;
    assert_eq!(
        h.post("/api/users", r#"{"name":"x"}"#).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_user_without_a_name_is_refused() {
    let (h, cookie) = signed_in().await;
    assert_eq!(
        h.post_with_cookie("/api/users", r#"{"name":"   "}"#, &cookie)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn listing_users_counts_their_passkeys() {
    let (h, cookie) = signed_in().await;
    let res = h.get_with_cookie("/api/users", &cookie).await;

    assert_eq!(res.status, StatusCode::OK);
    let users = res.json.as_array().unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0]["name"], "Saeed");
    assert_eq!(users[0]["credential_count"], 1);
}

#[tokio::test]
async fn a_fresh_link_can_be_issued_for_an_existing_user() {
    let (h, cookie) = signed_in().await;
    let id = h.get_with_cookie("/api/users", &cookie).await.json[0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let res = h
        .post_with_cookie(&format!("/api/users/{id}/enrollment"), "", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(
        res.json["enrollment_url"]
            .as_str()
            .unwrap()
            .contains("/enroll/")
    );
}

#[tokio::test]
async fn an_enrollment_link_for_an_unknown_user_is_404() {
    let (h, cookie) = signed_in().await;
    assert_eq!(
        h.post_with_cookie("/api/users/nobody/enrollment", "", &cookie)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_token_secret_is_returned_once_and_never_listed() {
    let (h, cookie) = signed_in().await;
    let created = h
        .post_with_cookie(
            "/api/tokens",
            r#"{"name":"agent","read_only":true}"#,
            &cookie,
        )
        .await;
    assert_eq!(created.status, StatusCode::OK);

    let secret = created.json["secret"].as_str().unwrap().to_string();
    assert!(secret.starts_with("ferr_"));
    assert_eq!(created.json["token"]["read_only"], true);

    let listed = h.get_with_cookie("/api/tokens", &cookie).await;
    assert!(
        !listed.json.to_string().contains(&secret),
        "the list endpoint leaked a token secret"
    );
}

#[tokio::test]
async fn a_minted_token_can_immediately_call_the_api() {
    let (h, cookie) = signed_in().await;
    let secret = h
        .post_with_cookie(
            "/api/tokens",
            r#"{"name":"agent","read_only":true}"#,
            &cookie,
        )
        .await
        .json["secret"]
        .as_str()
        .unwrap()
        .to_string();

    let me = h.get_with_bearer("/api/me", &secret).await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.json["kind"], "machine");
    assert_eq!(me.json["read_only"], true);
}

#[tokio::test]
async fn read_only_defaults_off_when_the_field_is_absent() {
    let (h, cookie) = signed_in().await;
    let created = h
        .post_with_cookie("/api/tokens", r#"{"name":"agent"}"#, &cookie)
        .await;
    assert_eq!(created.json["token"]["read_only"], false);
}

#[tokio::test]
async fn a_revoked_token_disappears_from_the_list() {
    let (h, cookie) = signed_in().await;
    let created = h
        .post_with_cookie("/api/tokens", r#"{"name":"agent"}"#, &cookie)
        .await;
    let id = created.json["token"]["id"].as_str().unwrap().to_string();

    assert_eq!(
        h.delete_with_cookie(&format!("/api/tokens/{id}"), &cookie)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert!(
        h.get_with_cookie("/api/tokens", &cookie)
            .await
            .json
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn token_management_requires_authentication() {
    let h = harness().await;
    assert_eq!(h.get("/api/tokens").await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        h.post("/api/tokens", r#"{"name":"x"}"#).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_user_lists_their_passkeys() {
    let (h, cookie) = signed_in().await;
    let users = h.get_with_cookie("/api/users", &cookie).await.json;
    let passkeys = users[0]["passkeys"].as_array().unwrap();

    assert_eq!(passkeys.len(), 1);
    assert!(passkeys[0]["added_at"].is_string());
    assert!(passkeys[0]["id"].is_string());
    assert!(
        !users.to_string().contains("\"credential\""),
        "the stored public key is not the panel's business: {users}"
    );
}

#[tokio::test]
async fn the_current_session_is_marked_as_current() {
    let (h, cookie) = signed_in().await;
    let sessions = h.get_with_cookie("/api/sessions", &cookie).await.json;
    let current: Vec<_> = sessions
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["current"] == true)
        .collect();

    assert_eq!(current.len(), 1, "exactly one session is the one asking");
    assert_eq!(current[0]["device"], "ferrum-tests");
}

#[tokio::test]
async fn one_user_never_sees_or_revokes_another_users_sessions() {
    let (h, cookie) = signed_in().await;
    let other = ferrum_core::users::create(&h.db, "Someone else")
        .await
        .unwrap();
    ferrum_core::sessions::issue(
        &h.db,
        &other.id,
        ferrum_core::sessions::Device {
            user_agent: Some("Their laptop"),
            ip: None,
        },
    )
    .await
    .unwrap();

    let listed = h.get_with_cookie("/api/sessions", &cookie).await.json;
    assert_eq!(
        listed.as_array().unwrap().len(),
        1,
        "one user must not see another's sessions: {listed}"
    );

    let theirs = ferrum_core::sessions::list_for(&h.db, &other.id)
        .await
        .unwrap();
    assert_eq!(
        h.delete_with_cookie(&format!("/api/sessions/{}", theirs[0].id), &cookie)
            .await
            .status,
        StatusCode::NOT_FOUND,
        "one user must not sign another out"
    );
    assert_eq!(
        ferrum_core::sessions::list_for(&h.db, &other.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn a_session_can_be_revoked_and_stops_working() {
    let (h, cookie) = signed_in().await;
    let second = ferrum_core::sessions::issue(
        &h.db,
        &ferrum_core::users::list(&h.db).await.unwrap()[0].id,
        ferrum_core::sessions::Device::default(),
    )
    .await
    .unwrap();

    let sessions = h.get_with_cookie("/api/sessions", &cookie).await.json;
    let other = sessions
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["current"] == false)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(
        h.delete_with_cookie(&format!("/api/sessions/{other}"), &cookie)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        h.get_with_cookie("/api/me", &second).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_token_prefix_identifies_it_without_revealing_it() {
    let (h, cookie) = signed_in().await;
    let created = h
        .post_with_cookie("/api/tokens", r#"{"name":"agent"}"#, &cookie)
        .await;
    let secret = created.json["secret"].as_str().unwrap().to_string();
    let prefix = created.json["token"]["prefix"]
        .as_str()
        .unwrap()
        .to_string();

    assert!(secret.starts_with(&prefix), "{prefix} vs {secret}");
    assert!(
        prefix.len() < 16,
        "a prefix must not be most of the secret: {prefix}"
    );

    let listed = h.get_with_cookie("/api/tokens", &cookie).await;
    assert_eq!(listed.json[0]["prefix"], prefix);
    assert!(!listed.json.to_string().contains(&secret));
}

#[tokio::test]
async fn a_machine_has_no_sessions_to_list() {
    let (h, cookie) = signed_in().await;
    let secret = h
        .post_with_cookie("/api/tokens", r#"{"name":"agent"}"#, &cookie)
        .await
        .json["secret"]
        .as_str()
        .unwrap()
        .to_string();

    let res = h.get_with_bearer("/api/sessions", &secret).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.json.as_array().unwrap().is_empty(), "{}", res.json);
}

#[tokio::test]
async fn session_management_requires_authentication() {
    let h = harness().await;
    assert_eq!(
        h.get("/api/sessions").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_read_only_token_can_still_reach_the_api_it_is_allowed() {
    let (h, cookie) = signed_in().await;
    let secret = h
        .post_with_cookie(
            "/api/tokens",
            r#"{"name":"agent","read_only":true}"#,
            &cookie,
        )
        .await
        .json["secret"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(
        h.get_with_bearer("/api/users", &secret).await.status,
        StatusCode::OK
    );
}
