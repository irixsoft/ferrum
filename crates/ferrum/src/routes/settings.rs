use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::State as Extract;
use axum::http::StatusCode;
use axum::{Json, Router, routing::get, routing::put};
use ferrum_core::host;
use ferrum_core::settings::{self, BuildLimits, SettingsError};
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/settings/builds", get(builds).put(set_builds))
        .route("/api/settings/checklist", put(set_checklist))
}

#[derive(Deserialize)]
struct Checklist {
    hidden: bool,
}

async fn set_checklist(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Checklist>,
) -> ApiResult<StatusCode> {
    host::set_checklist_hidden(&app.db, body.hidden).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct Builds {
    #[serde(flatten)]
    limits: BuildLimits,
    memory_total_mb: u64,
}

async fn current(app: &AppState) -> anyhow::Result<Builds> {
    let total_kb = app.platform.total_memory_kb().unwrap_or(0);
    Ok(Builds {
        limits: settings::build_limits(&app.db, app.platform.as_ref()).await?,
        memory_total_mb: settings::memory_ceiling_mb(total_kb),
    })
}

async fn builds(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Builds>> {
    Ok(Json(current(&app).await?))
}

async fn set_builds(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(limits): Json<BuildLimits>,
) -> ApiResult<Json<Builds>> {
    settings::set_build_limits(&app.db, app.platform.as_ref(), limits)
        .await
        .map_err(|e| match e.downcast_ref::<SettingsError>() {
            Some(_) => ApiError::bad_request(e.to_string()),
            None => e.into(),
        })?;
    Ok(Json(current(&app).await?))
}
