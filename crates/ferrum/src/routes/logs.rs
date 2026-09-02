use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, Query, State as Extract};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::get};
use ferrum_core::apps::{self, AppError};
use ferrum_core::logs::{self, DEFAULT_LINES, Line, Source};
use serde::Deserialize;
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/apps/{slug}/logs", get(read))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Request {
    source: Option<String>,
    lines: Option<u32>,
    follow: Option<String>,
}

/// A plain request answers the tail as JSON; `follow=1` answers `line` events until the client
/// leaves, which ends `journalctl` with it.
async fn read(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
    Query(request): Query<Request>,
) -> ApiResult<Response> {
    let found = apps::by_slug(&app.db, &slug)
        .await?
        .ok_or_else(|| ApiError::not_found(AppError::NotFound.to_string()))?;
    let source = match request.source.as_deref() {
        None => Source::App,
        Some(name) => Source::parse(name).ok_or_else(|| {
            ApiError::bad_request(format!("source must be app, access or error, not {name}."))
        })?,
    };
    let lines = request.lines.unwrap_or(DEFAULT_LINES);
    let follow = request
        .follow
        .as_deref()
        .is_some_and(|f| f == "1" || f == "true");
    if !follow {
        let tail = logs::tail(app.platform.as_ref(), &found, source, lines)?;
        return Ok(Json(tail).into_response());
    }
    if source != Source::App {
        return Err(ApiError::bad_request(
            "Only the application log can be followed; nginx logs are read as a tail.",
        ));
    }
    if !found.runtime.has_process() {
        return Err(ApiError::bad_request(
            "A static site has no process and no application log.",
        ));
    }
    let rx = logs::follow(app.platform.clone(), &found, lines);
    let stream = ReceiverStream::new(rx).map(|line| line_event(&line));
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response())
}

fn line_event(line: &Line) -> Result<Event, std::convert::Infallible> {
    Ok(Event::default()
        .event("line")
        .json_data(line)
        .expect("a log line serialises"))
}
