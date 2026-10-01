use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, State as Extract};
use axum::http::StatusCode;
use axum::{Json, Router, routing::delete, routing::get};
use ferrum_core::dns_providers::{self, NewProvider, Provider, ProviderError};
use serde::Deserialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/dns-providers", get(list).post(create))
        .route("/api/dns-providers/{id}", delete(remove))
}

fn refused(e: anyhow::Error) -> ApiError {
    match e.downcast_ref::<ProviderError>() {
        Some(ProviderError::Invalid(m)) => ApiError::bad_request(m.clone()),
        Some(ProviderError::InUse(m)) => ApiError::conflict(m.clone()),
        Some(ProviderError::NotFound) => ApiError::not_found(e.to_string()),
        None => e.into(),
    }
}

async fn list(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Vec<Provider>>> {
    Ok(Json(dns_providers::list(&app.db).await?))
}

#[derive(Deserialize)]
struct Create {
    #[serde(flatten)]
    provider: NewProvider,
    zone: String,
}

async fn create(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Create>,
) -> ApiResult<(StatusCode, Json<Provider>)> {
    let provider = dns_providers::create(&app.db, &app.http, body.provider, &body.zone)
        .await
        .map_err(refused)?;
    Ok((StatusCode::CREATED, Json(provider)))
}

async fn remove(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    dns_providers::remove(&app.db, &id).await.map_err(refused)?;
    Ok(StatusCode::NO_CONTENT)
}
