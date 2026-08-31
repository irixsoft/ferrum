use crate::auth::webauthn::{self, Pending};
use crate::auth::{cleared_session_cookie, cookie, session_cookie, user_agent};
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::State as Extract;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ferrum_core::{credentials, enrollment, sessions, setup, users};
use serde::{Deserialize, Serialize};
use webauthn_rs::prelude::*;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/register/begin", post(register_begin))
        .route("/api/auth/register/finish", post(register_finish))
        .route("/api/auth/login/begin", post(login_begin))
        .route("/api/auth/login/finish", post(login_finish))
        .route("/api/auth/logout", post(logout))
}

#[derive(Deserialize)]
pub struct RegisterBegin {
    pub enrollment: String,
}

#[derive(Serialize)]
pub struct RegisterBeginResponse {
    pub id: String,
    #[serde(flatten)]
    pub options: CreationChallengeResponse,
}

#[derive(Deserialize)]
pub struct RegisterFinish {
    pub id: String,
    pub enrollment: String,
    pub credential: RegisterPublicKeyCredential,
}

#[derive(Serialize)]
pub struct LoginBeginResponse {
    pub id: String,
    #[serde(flatten)]
    pub options: RequestChallengeResponse,
}

#[derive(Deserialize)]
pub struct LoginFinish {
    pub id: String,
    pub credential: PublicKeyCredential,
}

async fn instance(app: &AppState) -> ApiResult<Webauthn> {
    let hostname = setup::hostname(&app.db)
        .await?
        .ok_or_else(|| ApiError::unavailable("Ferrum is not set up yet. Run `ferrum setup`."))?;
    Ok(webauthn::instance(&hostname)?)
}

const UNKNOWN_LINK: &str = "That enrollment link is not valid. It may have been used already, or expired. Run `ferrum passkey enroll` for a new one.";

async fn register_begin(
    Extract(app): Extract<AppState>,
    Json(body): Json<RegisterBegin>,
) -> ApiResult<Json<RegisterBeginResponse>> {
    let user = enrollment::check(&app.db, &body.enrollment)
        .await?
        .ok_or_else(|| ApiError::unauthorized(UNKNOWN_LINK))?;

    let webauthn = instance(&app).await?;
    let (options, registration) = webauthn::start_registration(&webauthn, &user)?;

    let id = app.challenges.put(Pending::Register {
        user_id: user.id,
        state: Box::new(registration),
    });

    Ok(Json(RegisterBeginResponse { id, options }))
}

async fn register_finish(
    Extract(app): Extract<AppState>,
    headers: HeaderMap,
    Json(body): Json<RegisterFinish>,
) -> ApiResult<Response> {
    let Some(Pending::Register { user_id, state }) = app.challenges.take(&body.id) else {
        return Err(ApiError::bad_request(
            "That registration attempt expired. Open the enrollment link again.",
        ));
    };

    webauthn::assert_discoverable(&body.credential)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let user = enrollment::redeem(&app.db, &body.enrollment)
        .await?
        .ok_or_else(|| ApiError::unauthorized(UNKNOWN_LINK))?;
    if user.id != user_id {
        return Err(ApiError::bad_request(
            "That enrollment link belongs to a different account.",
        ));
    }

    let webauthn = instance(&app).await?;
    let passkey = webauthn
        .finish_passkey_registration(&body.credential, &state)
        .map_err(|e| ApiError::bad_request(format!("That passkey could not be verified: {e}")))?;

    let id = URL_SAFE_NO_PAD.encode(passkey.cred_id());
    let stored = serde_json::to_string(&passkey).map_err(anyhow::Error::from)?;
    credentials::save(&app.db, &user.id, &id, None, &stored)
        .await
        .map_err(|_| ApiError::conflict("That passkey is already registered."))?;

    issue_session(&app, &user.id, &headers).await
}

async fn login_begin(Extract(app): Extract<AppState>) -> ApiResult<Json<LoginBeginResponse>> {
    let webauthn = instance(&app).await?;
    let (options, authentication) = webauthn::start_login(&webauthn)?;
    let id = app.challenges.put(Pending::Login(Box::new(authentication)));
    Ok(Json(LoginBeginResponse { id, options }))
}

const REFUSED: &str = "That passkey was not accepted.";

async fn login_finish(
    Extract(app): Extract<AppState>,
    headers: HeaderMap,
    Json(body): Json<LoginFinish>,
) -> ApiResult<Response> {
    let Some(Pending::Login(state)) = app.challenges.take(&body.id) else {
        return Err(ApiError::bad_request(
            "That sign-in attempt expired. Try again.",
        ));
    };

    let webauthn = instance(&app).await?;
    let (handle, _) = webauthn
        .identify_discoverable_authentication(&body.credential)
        .map_err(|_| ApiError::unauthorized(REFUSED))?;

    let user = users::by_handle(&app.db, &handle.to_string())
        .await?
        .ok_or_else(|| ApiError::unauthorized(REFUSED))?;

    let stored = credentials::for_user(&app.db, &user.id).await?;
    let mut passkeys = Vec::with_capacity(stored.len());
    for row in &stored {
        let passkey: Passkey =
            serde_json::from_str(&row.credential).map_err(anyhow::Error::from)?;
        passkeys.push((row, passkey));
    }
    let keys: Vec<DiscoverableKey> = passkeys.iter().map(|(_, p)| p.into()).collect();

    let result = webauthn
        .finish_discoverable_authentication(&body.credential, *state, &keys)
        .map_err(|_| ApiError::unauthorized(REFUSED))?;

    let used = URL_SAFE_NO_PAD.encode(result.cred_id());
    let Some((row, passkey)) = passkeys.iter_mut().find(|(row, _)| row.id == used) else {
        return Err(ApiError::unauthorized(REFUSED));
    };

    if result.counter() > 0 && i64::from(result.counter()) <= row.counter {
        tracing::warn!(
            credential = %used,
            "signature counter did not advance; refusing a possible cloned authenticator"
        );
        return Err(ApiError::unauthorized(
            "That passkey's signature counter went backwards, which can mean it has been cloned. It has not been accepted.",
        ));
    }

    passkey.update_credential(&result);
    let updated = serde_json::to_string(&passkey).map_err(anyhow::Error::from)?;
    credentials::touch(&app.db, &row.id, result.counter(), &updated).await?;

    issue_session(&app, &user.id, &headers).await
}

async fn logout(Extract(app): Extract<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    if let Some(token) = cookie(&headers, sessions::COOKIE) {
        sessions::revoke_by_token(&app.db, &token).await?;
    }
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, cleared_session_cookie())],
    )
        .into_response())
}

async fn issue_session(app: &AppState, user_id: &str, headers: &HeaderMap) -> ApiResult<Response> {
    let token = sessions::issue(&app.db, user_id, user_agent(headers)).await?;
    Ok((
        StatusCode::NO_CONTENT,
        [(header::SET_COOKIE, session_cookie(&token))],
    )
        .into_response())
}
