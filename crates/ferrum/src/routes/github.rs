use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Query, State as Extract};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Json, Router, routing::get};
use ferrum_core::github::{self, manifest};
use ferrum_core::setup;
use serde::{Deserialize, Serialize};

const SETTINGS: &str = "/settings";
const EXPIRED: &str = "That connection attempt expired. Start again from Settings.";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/github/connect", axum::routing::post(connect))
        .route("/api/github/status", get(status))
        .route("/api/github", axum::routing::delete(remove))
}

pub fn public_router() -> Router<AppState> {
    Router::new().route("/api/github/callback", get(callback))
}

#[derive(Serialize)]
struct Handoff {
    manifest: serde_json::Value,
    state: String,
    action: String,
}

#[derive(Serialize)]
struct Status {
    connected: bool,
    #[serde(flatten, skip_serializing_if = "Option::is_none")]
    connection: Option<github::Connection>,
}

#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
}

async fn connect(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Handoff>> {
    let hostname = setup::hostname(&app.db)
        .await?
        .ok_or_else(|| ApiError::unavailable("This server has not finished setup yet."))?;

    let value = manifest::issue_state(&app.db).await?;
    Ok(Json(Handoff {
        manifest: manifest::manifest(&hostname),
        action: manifest::action(&value),
        state: value,
    }))
}

async fn callback(Extract(app): Extract<AppState>, Query(query): Query<Callback>) -> Response {
    match exchange(&app, query).await {
        Ok(()) => Redirect::to(&format!("{SETTINGS}?github=connected")).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn exchange(app: &AppState, query: Callback) -> ApiResult<()> {
    let state = query.state.unwrap_or_default();
    let code = query
        .code
        .filter(|c| !c.is_empty())
        .ok_or_else(|| ApiError::bad_request("GitHub did not send a code."))?;

    if !manifest::consume_state(&app.db, &state).await? {
        return Err(ApiError::bad_request(EXPIRED));
    }

    let connection = app.github.exchange(&code).await.map_err(|e| {
        tracing::error!(error = ?e, "the manifest exchange failed");
        ApiError::bad_request("GitHub refused the connection. Start again from Settings.")
    })?;
    github::save(&app.db, connection).await?;
    Ok(())
}

async fn status(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Status>> {
    let connection = github::load(&app.db).await?;
    Ok(Json(Status {
        connected: connection.is_some(),
        connection,
    }))
}

async fn remove(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<StatusCode> {
    github::disconnect(&app.db).await?;
    Ok(StatusCode::NO_CONTENT)
}
