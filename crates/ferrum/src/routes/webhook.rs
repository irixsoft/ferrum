use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::body::Bytes;
use axum::extract::State as Extract;
use axum::http::{HeaderMap, StatusCode};
use axum::{Router, routing::post};
use ferrum_core::github::{self, webhook};

const REFUSED: &str = "That delivery is not signed by a connected App.";

pub fn router() -> Router<AppState> {
    Router::new().route("/api/github/webhook", post(receive))
}

/// The body arrives as raw bytes because the signature covers exactly what github sent; a
/// re-serialised `Json<T>` differs in whitespace and key order and never verifies.
async fn receive(
    Extract(app): Extract<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<StatusCode> {
    let signature = header(&headers, webhook::SIGNATURE_HEADER).unwrap_or_default();
    let signer = github::webhook_secrets(&app.db)
        .await?
        .into_iter()
        .find(|(_, secret)| webhook::verify(secret, signature, &body))
        .map(|(app_id, _)| app_id)
        .ok_or_else(|| ApiError::unauthorized(REFUSED))?;

    let name = header(&headers, webhook::EVENT_HEADER)
        .ok_or_else(|| ApiError::bad_request("That delivery names no event."))?;
    let delivery = header(&headers, webhook::DELIVERY_HEADER)
        .ok_or_else(|| ApiError::bad_request("That delivery has no id."))?;

    let event = webhook::parse(name, &body)
        .map_err(|e| ApiError::bad_request(format!("That delivery could not be read: {e}")))?;

    if matches!(event, webhook::Event::Ping | webhook::Event::Other(_)) {
        return Ok(StatusCode::NO_CONTENT);
    }

    if webhook::record(&app.db, delivery, &event, &body).await? {
        let queued = app.deployer.react(&app.db, &event).await?;
        tracing::info!(
            event = event.name(),
            repository = event.repository(),
            app_id = signer,
            deploys = queued.len(),
            "delivery recorded"
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

fn header<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
}
