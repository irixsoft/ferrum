use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, State as Extract};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use ferrum_core::apps::vhost::{custom_path, vhost_path};
use ferrum_core::apps::{self, AppError};
use ferrum_core::nginx;
use ferrum_platform::PlatformError;
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new().route("/api/apps/{slug}/nginx", get(show).put(set_custom))
}

#[derive(Serialize)]
pub(crate) struct Files {
    managed: String,
    custom: String,
}

#[derive(Deserialize)]
struct Custom {
    custom: String,
}

async fn find(app: &AppState, slug: &str) -> ApiResult<apps::App> {
    apps::by_slug(&app.db, slug)
        .await?
        .ok_or_else(|| ApiError::not_found(AppError::NotFound.to_string()))
}

async fn show(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
) -> ApiResult<Json<Files>> {
    let found = find(&app, &slug).await?;
    Ok(Json(files(&app, &found)?))
}

pub(crate) fn files(app: &AppState, found: &apps::App) -> anyhow::Result<Files> {
    let read = |path: std::path::PathBuf| {
        app.platform
            .read_file(&path)
            .map(Option::unwrap_or_default)
            .map_err(anyhow::Error::from)
    };
    Ok(Files {
        managed: read(vhost_path(&found.slug))?,
        custom: read(custom_path(&found.slug))?,
    })
}

async fn set_custom(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
    Json(body): Json<Custom>,
) -> ApiResult<StatusCode> {
    let found = find(&app, &slug).await?;
    set_custom_file(&app, &found, &body.custom)?;
    Ok(StatusCode::NO_CONTENT)
}

/// `nginx -t` decides; a rejected file is put back as it was and nginx is never reloaded.
pub(crate) fn set_custom_file(app: &AppState, found: &apps::App, custom: &str) -> ApiResult<()> {
    nginx::replace_and_reload(app.platform.as_ref(), &custom_path(&found.slug), custom).map_err(
        |e| match e {
            PlatformError::Command { stderr, .. } => {
                ApiError::bad_request(format!("nginx rejected the file: {}", stderr.trim()))
            }
            other => ApiError::bad_request(format!("The host refused: {other}")),
        },
    )
}
