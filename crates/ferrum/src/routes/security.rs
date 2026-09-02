use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, State as Extract};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get, routing::post};
use ferrum_core::security::{self, Security, SecurityError, bans, firewall, ssh, updates};
use ferrum_core::setup;
use ferrum_platform::PlatformError;
use serde::Deserialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/security", get(status))
        .route("/api/security/firewall", post(enable_firewall))
        .route("/api/security/fail2ban", post(enable_fail2ban))
        .route("/api/security/updates", post(enable_updates))
        .route("/api/security/bans/{ip}/unban", post(unban))
        .route("/api/security/allowlist", post(allowlist))
        .route(
            "/api/security/ssh/disable-passwords",
            post(disable_passwords),
        )
}

async fn status(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Security>> {
    Ok(Json(
        security::status(&app.db, app.platform.as_ref())
            .await
            .map_err(security_error)?,
    ))
}

async fn enable_firewall(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<StatusCode> {
    firewall::enable(app.platform.as_ref()).map_err(security_error)?;
    Ok(StatusCode::ACCEPTED)
}

async fn enable_fail2ban(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<StatusCode> {
    bans::enable(&app.db, app.platform.as_ref())
        .await
        .map_err(security_error)?;
    Ok(StatusCode::ACCEPTED)
}

async fn enable_updates(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<StatusCode> {
    updates::enable(app.platform.as_ref()).map_err(security_error)?;
    Ok(StatusCode::ACCEPTED)
}

async fn unban(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(ip): Path<String>,
) -> ApiResult<StatusCode> {
    bans::unban(app.platform.as_ref(), &ip).map_err(security_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Address {
    ip: String,
}

async fn allowlist(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Address>,
) -> ApiResult<StatusCode> {
    bans::allow(&app.db, app.platform.as_ref(), &body.ip)
        .await
        .map_err(security_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Confirmation {
    name: String,
}

/// Typing the hostname is the confirmation; the keys are checked after, by the core.
async fn disable_passwords(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Confirmation>,
) -> ApiResult<StatusCode> {
    let hostname = setup::hostname(&app.db).await?.unwrap_or_default();
    if body.name.trim() != hostname {
        return Err(ApiError::bad_request(format!(
            "Type the hostname, {hostname}, to confirm."
        )));
    }
    ssh::disable_passwords(app.platform.as_ref()).map_err(security_error)?;
    Ok(StatusCode::ACCEPTED)
}

fn security_error(e: anyhow::Error) -> ApiError {
    match e.downcast_ref::<SecurityError>() {
        Some(SecurityError::AlreadyEnabled) => ApiError::conflict(e.to_string()),
        Some(SecurityError::NotBanned(_)) => ApiError::not_found(e.to_string()),
        Some(_) => ApiError::bad_request(e.to_string()),
        None => match e.downcast_ref::<PlatformError>() {
            Some(refused) => ApiError::bad_request(format!("The host refused: {refused}")),
            None => e.into(),
        },
    }
}
