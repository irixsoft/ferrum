use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, State as Extract};
use axum::{Json, Router, routing::get};
use ferrum_core::{credentials, enrollment, setup, users};
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/users", get(list).post(create))
        .route("/api/users/{id}/enrollment", axum::routing::post(reenroll))
}

#[derive(Serialize)]
pub struct UserSummary {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub credential_count: i64,
    pub passkeys: Vec<PasskeySummary>,
}

#[derive(Serialize)]
pub struct PasskeySummary {
    pub id: String,
    pub label: Option<String>,
    pub added_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Deserialize)]
pub struct NewUser {
    pub name: String,
}

#[derive(Serialize)]
pub struct Enrolled {
    pub user: UserSummary,
    pub enrollment_url: String,
}

#[derive(Serialize)]
pub struct EnrollmentLink {
    pub enrollment_url: String,
}

async fn list(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Vec<UserSummary>>> {
    let mut summaries = Vec::new();
    for user in users::list(&app.db).await? {
        summaries.push(summarise(&app, user).await?);
    }
    Ok(Json(summaries))
}

async fn create(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<NewUser>,
) -> ApiResult<Json<Enrolled>> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("A user needs a name."));
    }

    let user = users::create(&app.db, name).await?;
    let enrollment_url = issue_link(&app, &user.id).await?;

    Ok(Json(Enrolled {
        user: summarise(&app, user).await?,
        enrollment_url,
    }))
}

async fn reenroll(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(id): Path<String>,
) -> ApiResult<Json<EnrollmentLink>> {
    users::by_id(&app.db, &id)
        .await?
        .ok_or_else(|| ApiError::not_found("No such user."))?;

    Ok(Json(EnrollmentLink {
        enrollment_url: issue_link(&app, &id).await?,
    }))
}

async fn issue_link(app: &AppState, user_id: &str) -> ApiResult<String> {
    let hostname = setup::hostname(&app.db)
        .await?
        .ok_or_else(|| ApiError::unavailable("Ferrum is not set up yet."))?;
    let token = enrollment::issue(&app.db, user_id).await?;
    Ok(enrollment::url(&hostname, &token))
}

async fn summarise(app: &AppState, user: users::User) -> ApiResult<UserSummary> {
    let passkeys: Vec<PasskeySummary> = credentials::for_user(&app.db, &user.id)
        .await?
        .into_iter()
        .map(|c| PasskeySummary {
            id: c.id,
            label: c.label,
            added_at: c.created_at,
            last_used_at: c.last_used,
        })
        .collect();

    Ok(UserSummary {
        credential_count: passkeys.len() as i64,
        id: user.id,
        name: user.name,
        created_at: user.created_at,
        passkeys,
    })
}
