use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, State as Extract};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use ferrum_core::tokens;
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/tokens", get(list).post(create))
        .route("/api/tokens/{id}", axum::routing::delete(revoke))
}

#[derive(Serialize)]
pub struct TokenSummary {
    pub id: String,
    pub name: String,
    pub read_only: bool,
    pub created_at: String,
    pub last_used: Option<String>,
}

impl From<tokens::ApiToken> for TokenSummary {
    fn from(t: tokens::ApiToken) -> Self {
        Self {
            id: t.id,
            name: t.name,
            read_only: t.read_only,
            created_at: t.created_at,
            last_used: t.last_used,
        }
    }
}

#[derive(Deserialize)]
pub struct NewToken {
    pub name: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Serialize)]
pub struct Minted {
    pub token: TokenSummary,
    pub secret: String,
}

async fn list(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Vec<TokenSummary>>> {
    Ok(Json(
        tokens::list(&app.db)
            .await?
            .into_iter()
            .map(TokenSummary::from)
            .collect(),
    ))
}

async fn create(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<NewToken>,
) -> ApiResult<Json<Minted>> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("A token needs a name."));
    }

    let minted = tokens::mint(&app.db, name, body.read_only).await?;
    Ok(Json(Minted {
        token: minted.token.into(),
        secret: minted.secret,
    }))
}

async fn revoke(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    tokens::revoke(&app.db, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}
