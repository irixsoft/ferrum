use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, Query, State as Extract};
use axum::{Json, Router, routing::get};
use ferrum_core::apps::{self, AppError};
use ferrum_core::host::{self, HostStatus};
use ferrum_core::metrics::{self, HOST, POINTS};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const MB: f64 = 1024.0 * 1024.0;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/host", get(status))
        .route("/api/metrics", get(host_metrics))
        .route("/api/apps/{slug}/metrics", get(app_metrics))
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Window {
    range: Option<String>,
}

pub(crate) fn window_secs(range: Option<&str>) -> ApiResult<u64> {
    match range.unwrap_or("24h") {
        "1h" => Ok(3600),
        "24h" => Ok(86_400),
        "7d" => Ok(7 * 86_400),
        other => Err(ApiError::bad_request(format!(
            "range must be 1h, 24h or 7d, not {other}."
        ))),
    }
}

/// The panel's chart shape: `cpu` in percent, `memory` in percent of the host or MB of the app.
#[derive(Serialize)]
pub(crate) struct Series {
    t: Vec<i64>,
    values: BTreeMap<&'static str, Vec<f64>>,
}

fn shaped(series: metrics::Series, memory: impl Fn(f64) -> f64) -> Series {
    let mut values = BTreeMap::new();
    values.insert("cpu", series.values.get("cpu").cloned().unwrap_or_default());
    values.insert(
        "memory",
        series
            .values
            .get("memory_bytes")
            .map(|v| v.iter().map(|b| memory(*b)).collect())
            .unwrap_or_default(),
    );
    Series {
        t: series.t,
        values,
    }
}

async fn status(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<HostStatus>> {
    let build = crate::routes::version::build();
    Ok(Json(
        host::status(&app.db, app.platform.as_ref(), &build).await?,
    ))
}

async fn host_metrics(
    Extract(app): Extract<AppState>,
    _: Caller,
    Query(window): Query<Window>,
) -> ApiResult<Json<Series>> {
    let since = window_secs(window.range.as_deref())?;
    Ok(Json(host_series(&app, since).await?))
}

pub(crate) async fn host_series(app: &AppState, since: u64) -> anyhow::Result<Series> {
    let total = app.platform.proc_meminfo()?.total_kb as f64 * 1024.0;
    let series = metrics::series(&app.db, HOST, since, POINTS).await?;
    Ok(shaped(series, |bytes| {
        if total > 0.0 {
            (bytes / total * 1000.0).round() / 10.0
        } else {
            0.0
        }
    }))
}

async fn app_metrics(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
    Query(window): Query<Window>,
) -> ApiResult<Json<Series>> {
    let found = apps::by_slug(&app.db, &slug)
        .await?
        .ok_or_else(|| ApiError::not_found(AppError::NotFound.to_string()))?;
    let since = window_secs(window.range.as_deref())?;
    Ok(Json(app_series(&app, &found, since).await?))
}

pub(crate) async fn app_series(
    app: &AppState,
    found: &apps::App,
    since: u64,
) -> anyhow::Result<Series> {
    let series = metrics::series(&app.db, &found.id, since, POINTS).await?;
    Ok(shaped(series, |bytes| (bytes / MB * 10.0).round() / 10.0))
}
