use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, Query, State as Extract};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, KeepAliveStream, Sse};
use axum::{Json, Router, routing::get};
use ferrum_core::apps::{self, AppError};
use ferrum_core::deploy::log::{self, Event as LogEvent};
use ferrum_core::deploy::releases::Release;
use ferrum_core::deploy::{self, Deploy, Trigger, releases, snapshots};
use ferrum_core::github;
use serde::Deserialize;
use tokio_stream::wrappers::UnboundedReceiverStream;

const HISTORY: u32 = 50;
const RUNNING: &str = "A deploy is running for that application; wait for it to finish.";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/deploys", get(list))
        .route("/api/deploys/{id}", get(show))
        .route("/api/deploys/{id}/log", get(stream_log))
        .route("/api/deploys/{id}/cancel", axum::routing::post(cancel))
        .route("/api/apps/{slug}/deploys", get(list_for_app).post(create))
        .route("/api/apps/{slug}/releases", get(list_releases))
        .route("/api/apps/{slug}/rollback", axum::routing::post(rollback))
        .route("/api/snapshots/{id}/restore", axum::routing::post(restore))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Listing {
    running: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Manual {
    #[serde(rename = "ref")]
    git_ref: Option<String>,
    cli: bool,
}

#[derive(Deserialize)]
struct Rollback {
    release_id: String,
    #[serde(default)]
    restore_deploy_id: Option<String>,
}

async fn find_app(app: &AppState, slug: &str) -> ApiResult<apps::App> {
    apps::by_slug(&app.db, slug)
        .await?
        .ok_or_else(|| ApiError::not_found(AppError::NotFound.to_string()))
}

pub(crate) async fn find_deploy(app: &AppState, id: &str) -> ApiResult<Deploy> {
    deploy::by_id(&app.db, id)
        .await?
        .ok_or_else(|| ApiError::not_found("No such deploy."))
}

async fn list(
    Extract(app): Extract<AppState>,
    _: Caller,
    Query(listing): Query<Listing>,
) -> ApiResult<Json<serde_json::Value>> {
    if listing.running.is_some() {
        let running = deploy::running(&app.db).await?;
        return Ok(Json(
            serde_json::to_value(running).map_err(anyhow::Error::from)?,
        ));
    }
    let deploys = deploy::list(&app.db, None, HISTORY).await?;
    Ok(Json(
        serde_json::to_value(deploys).map_err(anyhow::Error::from)?,
    ))
}

async fn list_for_app(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Deploy>>> {
    let found = find_app(&app, &slug).await?;
    Ok(Json(deploy::list(&app.db, Some(&found.id), HISTORY).await?))
}

async fn show(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(id): Path<String>,
) -> ApiResult<Json<Deploy>> {
    Ok(Json(find_deploy(&app, &id).await?))
}

async fn create(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
    body: String,
) -> ApiResult<(StatusCode, Json<Deploy>)> {
    let found = find_app(&app, &slug).await?;
    let manual: Manual = if body.trim().is_empty() {
        Manual::default()
    } else {
        serde_json::from_str(&body).map_err(|e| ApiError::bad_request(e.to_string()))?
    };
    let trigger = if manual.cli {
        Trigger::Cli
    } else {
        Trigger::Manual
    };
    let queued = queue(&app, &found, manual.git_ref.as_deref(), trigger).await?;
    Ok((StatusCode::ACCEPTED, Json(queued)))
}

pub(crate) async fn queue(
    app: &AppState,
    found: &apps::App,
    git_ref: Option<&str>,
    trigger: Trigger,
) -> ApiResult<Deploy> {
    app.deployer
        .queue_ref(found, git_ref, trigger)
        .await
        .map_err(github_error)
}

pub(crate) async fn queue_rollback(
    app: &AppState,
    found: &apps::App,
    release_id: &str,
    restore_deploy_id: Option<&str>,
) -> ApiResult<Deploy> {
    let release = releases::by_id(&app.db, release_id)
        .await?
        .filter(|r| r.app_id == found.id)
        .ok_or_else(|| ApiError::not_found("No such release for this application."))?;
    if let Some(deploy_id) = restore_deploy_id {
        let snapshot_owner = find_deploy(app, deploy_id).await?;
        if snapshot_owner.app_id != found.id {
            return Err(ApiError::not_found("No such deploy for this application."));
        }
        if snapshots::for_deploy(&app.db, deploy_id).await?.is_empty() {
            return Err(ApiError::bad_request("That deploy took no snapshot."));
        }
    }
    Ok(app
        .deployer
        .enqueue_rollback(found, &release, restore_deploy_id)
        .await?)
}

fn github_error(e: anyhow::Error) -> ApiError {
    let message = e.to_string();
    if message == github::token::NOT_CONNECTED || message == github::token::NOT_INSTALLED {
        return ApiError::unavailable(message);
    }
    if message.starts_with("GitHub has no branch") || message.ends_with("no published release yet")
    {
        return ApiError::not_found(message);
    }
    e.into()
}

async fn cancel(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let found = find_deploy(&app, &id).await?;
    if found.state != Some(deploy::DeployState::Queued) {
        return Err(ApiError::conflict(
            "Only a queued deploy can be cancelled; this one has started.",
        ));
    }
    deploy::delete(&app.db, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_releases(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Release>>> {
    let found = find_app(&app, &slug).await?;
    Ok(Json(releases::for_app(&app.db, &found.id).await?))
}

async fn rollback(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
    Json(body): Json<Rollback>,
) -> ApiResult<(StatusCode, Json<Deploy>)> {
    let found = find_app(&app, &slug).await?;
    let queued = queue_rollback(
        &app,
        &found,
        &body.release_id,
        body.restore_deploy_id.as_deref(),
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(queued)))
}

async fn restore(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let snapshot = snapshots::by_id(&app.db, &id)
        .await?
        .ok_or_else(|| ApiError::not_found("No such snapshot."))?;
    if let Some(running) = deploy::running(&app.db).await?
        && let Some(owner) = snapshot.deploy_id.as_deref()
        && let Some(taken_by) = deploy::by_id(&app.db, owner).await?
        && taken_by.app_id == running.app_id
    {
        return Err(ApiError::conflict(RUNNING));
    }
    snapshots::restore(&app.db, app.platform.as_ref(), &id)
        .await
        .map_err(|e| ApiError::bad_request(format!("The restore failed: {e:#}")))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Every stored line, then live ones, then `done` with the outcome. The subscription is taken
/// before the stored lines are read so nothing falls between the two.
async fn stream_log(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(id): Path<String>,
) -> ApiResult<Sse<KeepAliveStream<UnboundedReceiverStream<Result<Event, std::convert::Infallible>>>>>
{
    let found = find_deploy(&app, &id).await?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut live = app.deployer.log().subscribe();
        let mut last_seq = 0;
        let stored = match log::lines(&app.db, &found.id, 0).await {
            Ok(lines) => lines,
            Err(e) => {
                tracing::error!(error = ?e, "reading a deploy log");
                return;
            }
        };
        for line in stored {
            last_seq = line.seq;
            if tx.send(line_event(&line)).is_err() {
                return;
            }
        }
        let finished = match deploy::by_id(&app.db, &found.id).await {
            Ok(Some(d)) => d.outcome,
            _ => Some(deploy::Outcome::Failed),
        };
        if let Some(outcome) = finished {
            let _ = tx.send(done_event(outcome));
            return;
        }
        loop {
            match live.recv().await {
                Ok(LogEvent::Line { deploy_id, line }) if deploy_id == found.id => {
                    if line.seq <= last_seq {
                        continue;
                    }
                    last_seq = line.seq;
                    if tx.send(line_event(&line)).is_err() {
                        return;
                    }
                }
                Ok(LogEvent::Done { deploy_id, outcome }) if deploy_id == found.id => {
                    let _ = tx.send(done_event(outcome));
                    return;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let Ok(missed) = log::lines(&app.db, &found.id, last_seq).await else {
                        return;
                    };
                    for line in missed {
                        last_seq = line.seq;
                        if tx.send(line_event(&line)).is_err() {
                            return;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    });
    Ok(Sse::new(UnboundedReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

fn line_event(line: &log::Line) -> Result<Event, std::convert::Infallible> {
    Ok(Event::default()
        .event("line")
        .json_data(line)
        .expect("a log line serialises"))
}

fn done_event(outcome: deploy::Outcome) -> Result<Event, std::convert::Infallible> {
    Ok(Event::default()
        .event("done")
        .json_data(serde_json::json!({ "outcome": outcome }))
        .expect("an outcome serialises"))
}
