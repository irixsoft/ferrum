use crate::auth::{Caller, cookie};
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, State as Extract};
use axum::http::{HeaderMap, StatusCode};
use axum::{Json, Router, routing::get};
use ferrum_core::sessions;
use serde::Serialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/sessions", get(list))
        .route("/api/sessions/{id}", axum::routing::delete(revoke))
}

#[derive(Serialize)]
pub struct SessionSummary {
    pub id: String,
    pub device: Option<String>,
    pub ip: Option<String>,
    pub started_at: String,
    pub last_seen: String,
    pub current: bool,
}

async fn list(
    Extract(app): Extract<AppState>,
    caller: Caller,
    headers: HeaderMap,
) -> ApiResult<Json<Vec<SessionSummary>>> {
    let Some(user) = caller.user() else {
        return Ok(Json(Vec::new()));
    };

    let presented = cookie(&headers, sessions::COOKIE);
    Ok(Json(
        sessions::list_for(&app.db, &user.id)
            .await?
            .into_iter()
            .map(|s| SessionSummary {
                current: presented
                    .as_deref()
                    .is_some_and(|token| sessions::is_current(token, &s.id)),
                id: s.id,
                device: s.user_agent,
                ip: s.ip,
                started_at: s.created_at,
                last_seen: s.last_seen,
            })
            .collect(),
    ))
}

async fn revoke(
    Extract(app): Extract<AppState>,
    caller: Caller,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    let user = caller
        .user()
        .ok_or_else(|| ApiError::not_found("No such session."))?;

    if sessions::revoke_for(&app.db, &user.id, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("No such session."))
    }
}
