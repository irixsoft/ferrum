use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, Query, State as Extract};
use axum::response::sse::{Event, Sse};
use axum::{Json, Router, routing::get};
use ferrum_core::runtime::toolchain::{self, Progress, Toolchain};
use ferrum_core::runtime::{self, RuntimeKind, Target, bun, dotnet, node};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::UnboundedReceiverStream;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/runtimes", get(list))
        .route("/api/runtimes/{kind}/resolve", get(resolve))
        .route(
            "/api/runtimes/{kind}/{version}",
            axum::routing::post(install),
        )
}

#[derive(Serialize)]
struct Runtimes {
    installed: Vec<Toolchain>,
    dotnet_channels: Vec<&'static str>,
}

#[derive(Deserialize)]
struct Wanted {
    version: Option<String>,
}

#[derive(Serialize)]
struct Resolved {
    version: String,
}

fn kind(name: &str) -> ApiResult<RuntimeKind> {
    RuntimeKind::parse(name)
        .filter(|k| k.installs_toolchain())
        .ok_or_else(|| ApiError::not_found(format!("{name} is not a runtime Ferrum installs.")))
}

async fn list(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Runtimes>> {
    Ok(Json(Runtimes {
        installed: toolchain::installed(&app.db).await?,
        dotnet_channels: dotnet::CHANNELS.to_vec(),
    }))
}

async fn resolve(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(name): Path<String>,
    Query(wanted): Query<Wanted>,
) -> ApiResult<Json<Resolved>> {
    let kind = kind(&name)?;
    let wanted = wanted
        .version
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let version = match kind {
        RuntimeKind::Node => node::resolve(&app.http, &app.mirrors.node_index_url(), wanted).await,
        RuntimeKind::Bun => bun::resolve(&app.github, wanted).await,
        RuntimeKind::Dotnet => Ok(dotnet::channel(wanted)),
        RuntimeKind::Static => unreachable!("filtered by kind()"),
    }
    .map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    Ok(Json(Resolved { version }))
}

async fn install(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path((name, version)): Path<(String, String)>,
) -> ApiResult<Sse<UnboundedReceiverStream<Result<Event, std::convert::Infallible>>>> {
    let kind = kind(&name)?;
    let runtime = runtime::by_kind(kind);
    if !runtime.valid_version(&version) {
        return Err(ApiError::bad_request(format!(
            "{version} is not a full {kind} version."
        )));
    }
    let target = Target::of(app.platform.as_ref())?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let events = tx.clone();
        let outcome = toolchain::ensure(
            &app.db,
            app.platform.as_ref(),
            &app.http,
            &app.toolchains,
            runtime,
            &version,
            target,
            &app.mirrors,
            move |p| {
                let _ = events.send(event(&p));
            },
        )
        .await;
        if let Err(e) = outcome {
            tracing::error!(error = ?e, %kind, %version, "toolchain install failed");
            let _ = tx.send(event(&Progress::Failed {
                error: format!("{e:#}"),
            }));
        }
    });

    Ok(Sse::new(UnboundedReceiverStream::new(rx)))
}

fn event(progress: &Progress) -> Result<Event, std::convert::Infallible> {
    Ok(Event::default()
        .json_data(progress)
        .expect("progress serialises"))
}
