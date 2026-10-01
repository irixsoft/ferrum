use crate::auth::Caller;
use crate::help::{self, Topic};
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::Path;
use axum::{Json, Router, routing::get};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/help", get(list))
        .route("/api/help/{slug}", get(show))
}

async fn list(_: Caller) -> Json<Vec<Topic>> {
    Json(help::list())
}

async fn show(_: Caller, Path(slug): Path<String>) -> ApiResult<Json<Topic>> {
    help::get(&slug)
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("There is no help topic called {slug}.")))
}
