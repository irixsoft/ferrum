use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::{AppState, Install};
use axum::extract::{Path, State as Extract};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use ferrum_core::apps::{self, provision};
use ferrum_core::postgres::{self, Database, DbError, NewDatabase};
use ferrum_core::{redis, setup};
use serde::{Deserialize, Serialize};

pub const NOT_INSTALLED: &str = "PostgreSQL is not installed yet.";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/postgres", get(status))
        .route("/api/postgres/install", axum::routing::post(install))
        .route("/api/databases", get(list).post(create))
        .route("/api/databases/{name}", get(show).delete(remove))
        .route(
            "/api/databases/{name}/extensions",
            axum::routing::post(enable_extension),
        )
        .route("/api/redis", get(redis_list))
}

#[derive(Serialize)]
struct Status {
    installed: bool,
    major: Option<u32>,
    installing: bool,
    error: Option<String>,
    tunnel: String,
    extensions: Vec<&'static str>,
}

#[derive(Deserialize)]
struct Create {
    #[serde(flatten)]
    database: NewDatabase,
    app_slug: Option<String>,
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

async fn status_of(app: &AppState) -> ApiResult<Status> {
    let pinned = postgres::major(&app.db).await?;
    let present = app.platform.postgres_major_installed();
    let (installing, error) = match &*app.postgres_install.lock().unwrap() {
        Install::Idle => (false, None),
        Install::Running => (true, None),
        Install::Failed(e) => (false, Some(e.clone())),
    };
    let hostname = setup::hostname(&app.db).await?.unwrap_or_default();
    Ok(Status {
        installed: present.is_some(),
        major: pinned.or(present),
        installing,
        error,
        tunnel: postgres::tunnel_command(&hostname),
        extensions: postgres::EXTENSIONS.iter().map(|(name, _)| *name).collect(),
    })
}

async fn status(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Status>> {
    Ok(Json(status_of(&app).await?))
}

/// apt takes a minute, so the install runs in the background and the status reports on it.
async fn install(
    Extract(app): Extract<AppState>,
    _: Caller,
) -> ApiResult<(StatusCode, Json<Status>)> {
    let started = {
        let mut state = app.postgres_install.lock().unwrap();
        if *state == Install::Running {
            false
        } else {
            *state = Install::Running;
            true
        }
    };
    if started {
        let task = app.clone();
        tokio::spawn(async move {
            let result =
                postgres::ensure_installed(&task.db, task.platform.as_ref(), &task.codename).await;
            *task.postgres_install.lock().unwrap() = match result {
                Ok(_) => Install::Idle,
                Err(e) => Install::Failed(e.to_string()),
            };
        });
    }
    Ok((StatusCode::ACCEPTED, Json(status_of(&app).await?)))
}

async fn list(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Vec<Database>>> {
    Ok(Json(postgres::list(&app.db, app.platform.as_ref()).await?))
}

async fn create(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Create>,
) -> ApiResult<(StatusCode, Json<Database>)> {
    if app.platform.postgres_major_installed().is_none() {
        return Err(ApiError::conflict(NOT_INSTALLED));
    }
    let linked_app = match &body.app_slug {
        Some(slug) => Some(
            apps::by_slug(&app.db, slug)
                .await?
                .ok_or_else(|| ApiError::not_found("No such application."))?,
        ),
        None => None,
    };
    let created = postgres::create(&app.db, app.platform.as_ref(), body.database)
        .await
        .map_err(db_error)?;
    if let Some(target) = linked_app {
        postgres::link(&app.db, &target.id, &created.name).await?;
        provision::write_env(&app.db, app.platform.as_ref(), &target).await?;
        return Ok((
            StatusCode::CREATED,
            Json(
                postgres::by_name(&app.db, &created.name)
                    .await?
                    .unwrap_or(created),
            ),
        ));
    }
    Ok((StatusCode::CREATED, Json(created)))
}

async fn show(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(name): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let found = postgres::list(&app.db, app.platform.as_ref())
        .await?
        .into_iter()
        .find(|d| d.name == name)
        .ok_or_else(|| ApiError::not_found(DbError::NotFound.to_string()))?;
    let mut value = serde_json::to_value(&found).map_err(anyhow::Error::from)?;
    value["url_hint"] =
        serde_json::Value::String(postgres::url(&found.name, &found.role, "<password>"));
    Ok(Json(value))
}

async fn remove(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(name): Path<String>,
    Json(body): Json<Named>,
) -> ApiResult<StatusCode> {
    if body.name.trim() != name {
        return Err(ApiError::bad_request(
            "Type the database's name exactly to delete it.",
        ));
    }
    postgres::delete(&app.db, app.platform.as_ref(), &name)
        .await
        .map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn enable_extension(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(name): Path<String>,
    Json(body): Json<Named>,
) -> ApiResult<StatusCode> {
    postgres::enable_extension(&app.db, app.platform.as_ref(), &name, &body.name)
        .await
        .map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn redis_list(
    Extract(app): Extract<AppState>,
    _: Caller,
) -> ApiResult<Json<Vec<redis::Listed>>> {
    Ok(Json(redis::list(&app.db).await?))
}

pub fn db_error(e: anyhow::Error) -> ApiError {
    match e.downcast_ref::<DbError>() {
        Some(DbError::Taken(_)) | Some(DbError::Linked(..)) => ApiError::conflict(e.to_string()),
        Some(DbError::NotFound) => ApiError::not_found(e.to_string()),
        Some(DbError::Invalid(_)) | Some(DbError::Host(_)) => ApiError::bad_request(e.to_string()),
        None => e.into(),
    }
}
