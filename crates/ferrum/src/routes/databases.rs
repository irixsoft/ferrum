use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::{AppState, Install};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Path, Request, State as Extract};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use ferrum_core::apps::{self, provision};
use ferrum_core::postgres::restore::{self, Format, Staged};
use ferrum_core::postgres::{self, Database, DbError, NewDatabase};
use ferrum_core::{redis, setup};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::PermissionsExt;
use tokio::io::AsyncWriteExt;
use tokio_stream::StreamExt;

pub const NOT_INSTALLED: &str = "PostgreSQL is not installed yet.";

const DISK_HEADROOM: u64 = 256 * 1024 * 1024;

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
        .route(
            "/api/databases/{name}/restore",
            axum::routing::post(restore_upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/redis", get(redis_list))
}

#[derive(Serialize)]
pub(crate) struct Listed {
    #[serde(flatten)]
    database: Database,
    restore: RestoreStatus,
}

#[derive(Serialize)]
pub(crate) struct RestoreStatus {
    running: bool,
    error: Option<String>,
}

fn restore_status(app: &AppState, name: &str) -> RestoreStatus {
    match app.restores.lock().unwrap().get(name) {
        Some(Install::Running) => RestoreStatus {
            running: true,
            error: None,
        },
        Some(Install::Failed(e)) => RestoreStatus {
            running: false,
            error: Some(e.clone()),
        },
        _ => RestoreStatus {
            running: false,
            error: None,
        },
    }
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

async fn list(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Vec<Listed>>> {
    Ok(Json(listed(&app).await?))
}

pub(crate) async fn listed(app: &AppState) -> ApiResult<Vec<Listed>> {
    Ok(postgres::list(&app.db, app.platform.as_ref())
        .await?
        .into_iter()
        .map(|database| Listed {
            restore: restore_status(app, &database.name),
            database,
        })
        .collect())
}

async fn create(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Create>,
) -> ApiResult<(StatusCode, Json<Database>)> {
    let created = create_database(&app, body.database, body.app_slug.as_deref()).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub(crate) async fn create_database(
    app: &AppState,
    new: NewDatabase,
    app_slug: Option<&str>,
) -> ApiResult<Database> {
    if app.platform.postgres_major_installed().is_none() {
        return Err(ApiError::conflict(NOT_INSTALLED));
    }
    let linked_app = match app_slug {
        Some(slug) => Some(
            apps::by_slug(&app.db, slug)
                .await?
                .ok_or_else(|| ApiError::not_found("No such application."))?,
        ),
        None => None,
    };
    let created = postgres::create(&app.db, app.platform.as_ref(), new)
        .await
        .map_err(db_error)?;
    if let Some(target) = linked_app {
        postgres::link(&app.db, &target.id, &created.name).await?;
        provision::write_env(&app.db, app.platform.as_ref(), &target).await?;
        return Ok(postgres::by_name(&app.db, &created.name)
            .await?
            .unwrap_or(created));
    }
    Ok(created)
}

async fn show(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(name): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(detail(&app, &name).await?))
}

/// The URL carries `<password>` where the secret goes; the real one is only ever in `.env`.
pub(crate) async fn detail(app: &AppState, name: &str) -> ApiResult<serde_json::Value> {
    let found = postgres::list(&app.db, app.platform.as_ref())
        .await?
        .into_iter()
        .find(|d| d.name == name)
        .ok_or_else(|| ApiError::not_found(DbError::NotFound.to_string()))?;
    let mut value = serde_json::to_value(&found).map_err(anyhow::Error::from)?;
    value["url_hint"] =
        serde_json::Value::String(postgres::url(&found.name, &found.role, "<password>"));
    value["restore"] =
        serde_json::to_value(restore_status(app, name)).map_err(anyhow::Error::from)?;
    Ok(value)
}

/// Streamed to disk and refused early — five bytes for a gzip, the first byte past the free space.
async fn restore_upload(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(name): Path<String>,
    request: Request,
) -> ApiResult<(StatusCode, Json<serde_json::Value>)> {
    if app.platform.postgres_major_installed().is_none() {
        return Err(ApiError::conflict(NOT_INSTALLED));
    }
    if postgres::by_name(&app.db, &name).await?.is_none() {
        return Err(ApiError::not_found(DbError::NotFound.to_string()));
    }
    if app.restores.lock().unwrap().get(&name) == Some(&Install::Running) {
        return Err(ApiError::conflict(format!(
            "A restore of {name} is already running."
        )));
    }
    let staged = Staged::new(&app.db.data_dir, &name);
    let format = receive(&app, &staged, request.into_body()).await?;

    app.restores
        .lock()
        .unwrap()
        .insert(name.clone(), Install::Running);
    let task = app.clone();
    let database = name.clone();
    tokio::spawn(async move {
        let result =
            restore::restore(&task.db, task.platform.as_ref(), &database, &staged, format).await;
        drop(staged);
        let outcome = match result {
            Ok(()) => Install::Idle,
            Err(e) => Install::Failed(e.to_string()),
        };
        task.restores.lock().unwrap().insert(database, outcome);
    });
    Ok((StatusCode::ACCEPTED, Json(detail(&app, &name).await?)))
}

async fn receive(app: &AppState, staged: &Staged, body: Body) -> ApiResult<Format> {
    std::fs::create_dir_all(&staged.dir).map_err(anyhow::Error::from)?;
    std::fs::set_permissions(&staged.dir, std::fs::Permissions::from_mode(0o700))
        .map_err(anyhow::Error::from)?;
    let free = app
        .platform
        .disk_free_bytes(&app.db.data_dir)
        .map_err(anyhow::Error::from)?;
    let budget = free.saturating_sub(DISK_HEADROOM);
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&staged.path)
        .await
        .map_err(anyhow::Error::from)?;

    let mut stream = body.into_data_stream();
    let mut head = Vec::with_capacity(restore::SNIFF_LEN);
    let mut written = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| ApiError::bad_request(format!("The upload stopped early: {e}")))?;
        if head.len() < restore::SNIFF_LEN {
            let take = chunk.len().min(restore::SNIFF_LEN - head.len());
            head.extend_from_slice(&chunk[..take]);
            if head.len() == restore::SNIFF_LEN {
                restore::sniff(&head).map_err(|e| db_error(e.into()))?;
            }
        }
        written += chunk.len() as u64;
        if written > budget {
            return Err(ApiError::new(
                StatusCode::INSUFFICIENT_STORAGE,
                format!(
                    "That dump does not fit: {} MB free on this server.",
                    free / (1024 * 1024)
                ),
            ));
        }
        file.write_all(&chunk).await.map_err(anyhow::Error::from)?;
    }
    file.flush().await.map_err(anyhow::Error::from)?;
    restore::sniff(&head).map_err(|e| db_error(e.into()))
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
