use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::body::Bytes;
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
        .route("/api/github/{app_id}", axum::routing::delete(remove))
}

pub fn public_router() -> Router<AppState> {
    Router::new().route("/api/github/callback", get(callback))
}

#[derive(Deserialize, Default)]
struct Connect {
    organization: Option<String>,
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
    connections: Vec<github::Connection>,
}

#[derive(Deserialize)]
struct Callback {
    code: Option<String>,
    state: Option<String>,
}

/// The body is optional: the personal App needs nothing, an organisation names itself.
async fn connect(
    Extract(app): Extract<AppState>,
    _: Caller,
    body: Bytes,
) -> ApiResult<Json<Handoff>> {
    let wanted: Connect = if body.trim_ascii().is_empty() {
        Connect::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| ApiError::bad_request(format!("The request could not be read: {e}")))?
    };
    let organization = wanted
        .organization
        .as_deref()
        .map(str::trim)
        .filter(|o| !o.is_empty());
    if let Some(org) = organization
        && !manifest::valid_organization(org)
    {
        return Err(ApiError::bad_request(
            "An organisation is its GitHub login: letters, digits and hyphens.",
        ));
    }
    let hostname = setup::hostname(&app.db)
        .await?
        .ok_or_else(|| ApiError::unavailable("This server has not finished setup yet."))?;

    let value = manifest::issue_state(&app.db).await?;
    Ok(Json(Handoff {
        manifest: manifest::manifest(&hostname, organization),
        action: manifest::action(&value, organization),
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
    let connections = github::load_all(&app.db).await?;
    Ok(Json(Status {
        connected: !connections.is_empty(),
        connections,
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
pub(crate) fn reachable(e: anyhow::Error) -> ApiError {
    let message = format!("{e}");
    if github::token::user_fixable(&message) {
        return ApiError::unavailable(message);
    }
    e.into()
}

async fn remove(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(app_id): Path<i64>,
) -> ApiResult<StatusCode> {
    if !github::disconnect(&app.db, app_id).await? {
        return Err(ApiError::not_found("No such connection."));
    }
    app.github.forget();
    Ok(StatusCode::NO_CONTENT)
}
