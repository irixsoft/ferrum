mod support;

use axum::http::StatusCode;
use ferrum_core::{enrollment, users};
use support::*;

#[tokio::test]
async fn register_begin_rejects_an_unknown_enrollment_link() {
    let h = harness().await;
    let res = h
        .post("/api/auth/register/begin", r#"{"enrollment":"nope"}"#)
        .await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn register_begin_offers_a_discoverable_credential() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let res = h
        .post(
            "/api/auth/register/begin",
            &format!(r#"{{"enrollment":"{link}"}}"#),
        )
        .await;

    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(
        res.json["publicKey"]["authenticatorSelection"]["residentKey"],
        "required"
    );
    assert!(res.json["id"].is_string(), "a challenge id must come back");
}

#[tokio::test]
async fn login_begin_sends_an_empty_allow_credentials_list() {
    let h = harness().await;
    let res = h.post("/api/auth/login/begin", "{}").await;
    assert_eq!(res.status, StatusCode::OK);

    let allow = &res.json["publicKey"]["allowCredentials"];
    assert!(
        allow.is_null() || allow.as_array().is_some_and(|a| a.is_empty()),
        "allowCredentials must be empty so the browser picker supplies the identity: {}",
        res.json
    );
}

#[tokio::test]
async fn a_passkey_registers_and_then_signs_in() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();

    let registered = h.register(&mut key, &link).await;
    assert_eq!(registered.status, StatusCode::NO_CONTENT);
    let cookie = registered
        .session_cookie()
        .expect("registration signs you in");

    let signed_in = h.login(&mut key).await;
    assert_eq!(signed_in.status, StatusCode::NO_CONTENT);
    assert!(signed_in.session_cookie().is_some());
    assert_ne!(
        cookie,
        signed_in.session_cookie().unwrap(),
        "each sign-in must mint a fresh session"
    );
}

#[tokio::test]
async fn an_enrollment_link_cannot_attach_a_second_passkey() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;

    let mut first = soft_passkey();
    assert_eq!(
        h.register(&mut first, &link).await.status,
        StatusCode::NO_CONTENT
    );

    let mut second = soft_passkey();
    let replayed = h.register(&mut second, &link).await;
    assert_eq!(
        replayed.status,
        StatusCode::UNAUTHORIZED,
        "a used link must not enrol another authenticator"
    );
}

#[tokio::test]
async fn a_passkey_for_a_deleted_user_is_refused() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    h.register(&mut key, &link).await;

    let assertion = h.assertion(&mut key).await;
    sqlx::query("DELETE FROM users")
        .execute(&h.db.pool)
        .await
        .unwrap();

    assert_eq!(
        h.login_with(assertion).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn a_revoked_passkey_cannot_sign_in() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    h.register(&mut key, &link).await;

    let assertion = h.assertion(&mut key).await;
    sqlx::query("DELETE FROM credentials")
        .execute(&h.db.pool)
        .await
        .unwrap();

    assert_eq!(
        h.login_with(assertion).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_assertion_pointing_at_another_account_is_refused() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    h.register(&mut key, &link).await;

    let stranger = users::create(&h.db, "Someone else").await.unwrap();
    let mut assertion = h.assertion(&mut key).await;
    assertion.credential.response.user_handle = Some(handle_bytes(&stranger.handle));

    assert_eq!(
        h.login_with(assertion).await.status,
        StatusCode::UNAUTHORIZED,
        "a passkey must not authenticate an account it was not registered to"
    );
}

#[tokio::test]
async fn a_replayed_assertion_is_refused() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    h.register(&mut key, &link).await;

    let assertion = h.assertion(&mut key).await;
    let replay = Assertion {
        id: assertion.id.clone(),
        credential: assertion.credential.clone(),
    };

    assert_eq!(h.login_with(assertion).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        h.login_with(replay).await.status,
        StatusCode::BAD_REQUEST,
        "a challenge must be usable exactly once"
    );
}

#[tokio::test]
async fn a_stale_challenge_id_is_refused() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();

    let begun = h.register_begin(&link).await;
    let credential = key
        .register(&h.origin(), &begun)
        .expect("soft passkey registers");
    let body = serde_json::json!({
        "id": "never-issued",
        "enrollment": link,
        "credential": credential,
    });

    let res = h.post("/api/auth/register/finish", &body.to_string()).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_signature_counter_that_does_not_advance_is_refused() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    h.register(&mut key, &link).await;

    assert_eq!(h.login(&mut key).await.status, StatusCode::NO_CONTENT);

    sqlx::query("UPDATE credentials SET counter = 9999")
        .execute(&h.db.pool)
        .await
        .unwrap();

    assert_eq!(
        h.login(&mut key).await.status,
        StatusCode::UNAUTHORIZED,
        "a counter that goes backwards is the cloned-authenticator signal"
    );
}

#[tokio::test]
async fn a_successful_sign_in_advances_the_stored_counter() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    h.register(&mut key, &link).await;

    h.login(&mut key).await;
    let first: i64 = sqlx::query_scalar("SELECT counter FROM credentials")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();

    h.login(&mut key).await;
    let second: i64 = sqlx::query_scalar("SELECT counter FROM credentials")
        .fetch_one(&h.db.pool)
        .await
        .unwrap();

    assert!(second > first, "{second} should be past {first}");
}

#[tokio::test]
async fn signing_out_revokes_the_session() {
    let h = harness().await;
    let link = h.enrollment("Saeed").await;
    let mut key = soft_passkey();
    let cookie = h.register(&mut key, &link).await.session_cookie().unwrap();

    let out = h.post_with_cookie("/api/auth/logout", "", &cookie).await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);

    let user = users::list(&h.db).await.unwrap().pop().unwrap();
    assert!(
        ferrum_core::sessions::list_for(&h.db, &user.id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn signing_out_without_a_session_is_still_fine() {
    let h = harness().await;
    assert_eq!(
        h.post("/api/auth/logout", "").await.status,
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn auth_is_unavailable_before_setup_names_a_hostname() {
    let h = harness_without_hostname(NO_GITHUB).await;
    let user = users::create(&h.db, "Saeed").await.unwrap();
    let link = enrollment::issue(&h.db, &user.id).await.unwrap();

    let res = h
        .post(
            "/api/auth/register/begin",
            &format!(r#"{{"enrollment":"{link}"}}"#),
        )
        .await;
    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE);
}
