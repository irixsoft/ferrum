use crate::auth::Caller;
use crate::routes::apps::find;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, State as Extract};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, KeepAliveStream, Sse};
use axum::{Json, Router, routing::get};
use ferrum_core::apps::commands::{self, CommandError, Run};
use ferrum_core::deploy::log::{self, Event as LogEvent};
use serde::Deserialize;
use tokio_stream::wrappers::UnboundedReceiverStream;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps/{slug}/commands", get(list).post(start))
        .route("/api/commands/{id}/log", get(stream_log))
}

#[derive(Deserialize)]
struct Command {
    command: String,
}

async fn list(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
) -> ApiResult<Json<Vec<Run>>> {
    let found = find(&app, &slug).await?;
    Ok(Json(commands::list(&app.db, &found.id).await?))
}

async fn start(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
    Json(body): Json<Command>,
) -> ApiResult<(StatusCode, Json<Run>)> {
    let found = find(&app, &slug).await?;
    let run = commands::start(app.deployer.ctx(), &found, &body.command)
        .await
        .map_err(|e| match e.downcast_ref::<CommandError>() {
            Some(CommandError::Blank | CommandError::NotDeployed) => {
                ApiError::bad_request(e.to_string())
            }
            Some(CommandError::DeployRunning | CommandError::RunOpen) => {
                ApiError::conflict(e.to_string())
            }
            None => ApiError::from(e),
        })?;
    Ok((StatusCode::ACCEPTED, Json(run)))
}

/// Every stored line, then live ones, then `done` with the exit. The subscription is taken
/// before the stored lines are read so nothing falls between the two.
async fn stream_log(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(id): Path<String>,
) -> ApiResult<Sse<KeepAliveStream<UnboundedReceiverStream<Result<Event, std::convert::Infallible>>>>>
{
    let run = commands::by_id(&app.db, &id)
        .await?
        .ok_or_else(|| ApiError::not_found("No such command run."))?;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut live = app.deployer.log().subscribe();
        let mut last_seq = 0;
        let stored = match log::run_lines(&app.db, &run.id, 0).await {
            Ok(lines) => lines,
            Err(e) => {
                tracing::error!(error = ?e, "reading a command log");
                return;
            }
        };
        for line in stored {
            last_seq = line.seq;
            if tx.send(line_event(&line)).is_err() {
                return;
            }
        }
        let finished = match commands::by_id(&app.db, &run.id).await {
            Ok(Some(r)) => r.exit,
            _ => Some("Ferrum lost track of the run.".to_string()),
        };
        if let Some(exit) = finished {
            let _ = tx.send(done_event(&exit));
            return;
        }
        loop {
            match live.recv().await {
                Ok(LogEvent::RunLine { run_id, line }) if run_id == run.id => {
                    if line.seq <= last_seq {
                        continue;
                    }
                    last_seq = line.seq;
                    if tx.send(line_event(&line)).is_err() {
                        return;
                    }
                }
                Ok(LogEvent::RunDone { run_id, exit }) if run_id == run.id => {
                    let _ = tx.send(done_event(&exit));
                    return;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let Ok(missed) = log::run_lines(&app.db, &run.id, last_seq).await else {
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

fn done_event(exit: &str) -> Result<Event, std::convert::Infallible> {
    Ok(Event::default()
        .event("done")
        .json_data(serde_json::json!({ "exit": exit }))
        .expect("an exit serialises"))
}
