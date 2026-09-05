use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, Query, State as Extract};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Json, Router, routing::get};
use ferrum_core::github::{self, manifest};
use ferrum_core::setup;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::{Deserialize, Serialize};

const SETTINGS: &str = "/settings";
const EXPIRED: &str = "That connection attempt expired. Start again from Settings.";
const QUERY: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'&')
    .add(b'+')
    .add(b'<')
    .add(b'>')
    .add(b'=')
    .add(b'?');

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/github/connect", axum::routing::post(connect))
        .route("/api/github/status", get(status))
        .route("/api/github/repos", get(repos))
        .route("/api/github/repos/{owner}/{repo}/tags", get(tags))
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
        Err(e) if e.status.is_client_error() => Redirect::to(&format!(
            "{SETTINGS}?github=failed&reason={}",
            utf8_percent_encode(&e.message, QUERY)
        ))
        .into_response(),
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
    app.github.forget();
    Ok(())
}

async fn status(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Status>> {
    let connection = github::load(&app.db).await?;
    Ok(Json(Status {
        connected: connection.is_some(),
        connection,
    }))
}

async fn repos(
    Extract(app): Extract<AppState>,
    _: Caller,
) -> ApiResult<Json<Vec<github::repos::Repo>>> {
    app.github.repos(&app.db).await.map(Json).map_err(reachable)
}

async fn tags(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path((owner, repo)): Path<(String, String)>,
) -> ApiResult<Json<Vec<github::refs::Tag>>> {
    app.github
        .tags(&app.db, &format!("{owner}/{repo}"))
        .await
        .map(Json)
        .map_err(reachable)
}

/// "Not connected" and "not installed" are the user's to fix, so they must survive as a sentence
/// rather than collapsing into a 500.
fn reachable(e: anyhow::Error) -> ApiError {
    let message = format!("{e}");
    if message == github::token::NOT_CONNECTED || message == github::token::NOT_INSTALLED {
        return ApiError::unavailable(message);
    }
    e.into()
}

async fn remove(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<StatusCode> {
    github::disconnect(&app.db).await?;
    app.github.forget();
    Ok(StatusCode::NO_CONTENT)
}
