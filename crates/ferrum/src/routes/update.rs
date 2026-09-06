use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::State as Extract;
use axum::http::StatusCode;
use axum::{Json, Router, routing::get, routing::post, routing::put};
use ferrum_core::update::{Status, UpdateError, apply, check};
use serde::Deserialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/update", get(status).post(start))
        .route("/api/update/check", post(check_now))
        .route("/api/settings/updates", put(set_auto))
}

fn refused(e: UpdateError) -> ApiError {
    match e {
        UpdateError::NotNewer(_) | UpdateError::InProgress | UpdateError::Restarting(_) => {
            ApiError::conflict(e.to_string())
        }
        _ => ApiError::bad_request(e.to_string()),
    }
}

async fn status(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Status>> {
    Ok(Json(app.updater.status().await?))
}

async fn check_now(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Status>> {
    let status = app
        .updater
        .check()
        .await
        .map_err(|e| match e.downcast::<UpdateError>() {
            Ok(known) => refused(known),
            Err(other) => ApiError::unavailable(format!(
                "Could not check for a release: {}",
                apply::describe(&other)
            )),
        })?;
    Ok(Json(status))
}

async fn start(
    Extract(app): Extract<AppState>,
    _: Caller,
) -> ApiResult<(StatusCode, Json<Status>)> {
    let status = app.updater.status().await?;
    let latest = status
        .latest
        .filter(|_| status.available)
        .ok_or_else(|| refused(UpdateError::NotNewer(status.current.clone())))?;
    app.updater.start(latest).map_err(refused)?;
    Ok((StatusCode::ACCEPTED, Json(app.updater.status().await?)))
}

#[derive(Deserialize)]
struct Auto {
    auto: bool,
}

async fn set_auto(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Auto>,
) -> ApiResult<Json<Status>> {
    check::set_auto(&app.db, body.auto).await?;
    Ok(Json(app.updater.status().await?))
}
