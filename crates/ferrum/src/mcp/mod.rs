mod handler;
mod read;
mod write;

pub use handler::{Ferrum, READ_ONLY};

use crate::auth;
use crate::server::{AppState, LISTEN_ADDR};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Json, Response};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use std::sync::Arc;

pub const TOKEN_REQUIRED: &str = "An API token is required.";

/// Bearer only. A session cookie is never enough here, so a page in the browser cannot be made
/// into an agent.
pub async fn require_token(
    State(app): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let mut headers = HeaderMap::new();
    if let Some(value) = request.headers().get(header::AUTHORIZATION) {
        headers.insert(header::AUTHORIZATION, value.clone());
    }
    if auth::bearer(&headers).is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({ "error": TOKEN_REQUIRED })),
        )
            .into_response();
    }
    match auth::resolve(&app, &headers).await {
        Ok(caller) => {
            request.extensions_mut().insert(caller);
            next.run(request).await
        }
        Err(e) => e.into_response(),
    }
}

pub fn allowed_hosts(hostname: Option<&str>) -> Vec<String> {
    let mut hosts = vec![
        LISTEN_ADDR.to_string(),
        format!("localhost:{}", LISTEN_ADDR.port()),
    ];
    hosts.extend(hostname.map(str::to_string));
    hosts
}

pub fn service(state: AppState) -> StreamableHttpService<Ferrum, LocalSessionManager> {
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(allowed_hosts(state.hostname.as_deref()))
        .with_stateful_mode(false)
        .with_json_response(true);
    StreamableHttpService::new(
        move || Ok(Ferrum::new(state.clone())),
        Arc::new(LocalSessionManager::default()),
        config,
    )
}
