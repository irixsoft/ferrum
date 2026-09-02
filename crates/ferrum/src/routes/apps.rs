use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, State as Extract};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use ferrum_core::apps::{self, AppChanges, AppError, NewApp, env, provision};
use ferrum_core::detect::{self, DetectError, Detected};
use ferrum_core::redis::{self, RedisError};
use ferrum_core::runtime::toolchain;
use ferrum_core::{github, postgres};
use serde::Deserialize;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/apps", get(list).post(create))
        .route("/api/apps/detect", axum::routing::post(inspect))
        .route("/api/apps/{slug}", get(show).patch(update).delete(remove))
        .route("/api/apps/{slug}/env", axum::routing::put(set_env))
        .route(
            "/api/apps/{slug}/databases/{name}",
            axum::routing::post(link_database).delete(unlink_database),
        )
        .route(
            "/api/apps/{slug}/redis",
            axum::routing::post(request_redis).delete(release_redis),
        )
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RedisRequest {
    maxmemory_mb: Option<u32>,
}

#[derive(Deserialize)]
struct Inspect {
    repository: String,
    #[serde(rename = "ref")]
    git_ref: String,
    #[serde(default)]
    root: String,
}

#[derive(Deserialize)]
struct Removal {
    name: String,
}

async fn list(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Vec<apps::App>>> {
    Ok(Json(apps::list(&app.db).await?))
}

async fn inspect(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Inspect>,
) -> ApiResult<Json<Detected>> {
    detect::inspect(
        &app.github,
        &app.db,
        &body.repository,
        &body.git_ref,
        &body.root,
    )
    .await
    .map(Json)
    .map_err(|e| {
        if e.downcast_ref::<DetectError>().is_some() {
            return ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string());
        }
        let message = e.to_string();
        if message == github::token::NOT_CONNECTED || message == github::token::NOT_INSTALLED {
            return ApiError::unavailable(message);
        }
        if message.starts_with("GitHub has no branch or tag") {
            return ApiError::not_found(message);
        }
        e.into()
    })
}

async fn create(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(new): Json<NewApp>,
) -> ApiResult<(StatusCode, Json<apps::App>)> {
    apps::validate(&new).map_err(|e| app_error(e.into()))?;
    if toolchain::find(&app.db, new.toolchain, &new.runtime_version)
        .await?
        .is_none()
    {
        return Err(ApiError::conflict(format!(
            "{} {} is not installed yet.",
            new.toolchain, new.runtime_version
        )));
    }

    let resolved: Vec<String> = new
        .packages
        .iter()
        .flat_map(|p| app.platform.resolve_package(p))
        .collect();
    if !resolved.is_empty() {
        let names: Vec<&str> = resolved.iter().map(String::as_str).collect();
        app.platform
            .install_packages(&names)
            .map_err(|e| ApiError::bad_request(format!("Installing packages failed: {e}")))?;
    }

    let slug = new.slug.clone();
    let created = apps::create(&app.db, new).await.map_err(app_error)?;
    if let Err(e) = provision::provision(&app.db, app.platform.as_ref(), &created).await {
        apps::delete(&app.db, &slug).await?;
        return Err(ApiError::bad_request(format!(
            "The host could not be prepared: {e:#}"
        )));
    }
    Ok((StatusCode::CREATED, Json(created)))
}

async fn show(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let found = apps::by_slug(&app.db, &slug)
        .await?
        .ok_or_else(|| ApiError::not_found(AppError::NotFound.to_string()))?;
    let keys: Vec<serde_json::Value> = env::keys(&app.db, &found.id)
        .await?
        .into_iter()
        .map(|key| serde_json::json!({ "key": key }))
        .collect();
    let databases = postgres::names_for(&app.db, &found.id).await?;
    let instance = redis::for_app(&app.db, &found.id).await?;
    let managed = env::managed_for(&app.db, &found).await?.keys();
    let mut value = serde_json::to_value(&found).map_err(anyhow::Error::from)?;
    value["env"] = serde_json::Value::Array(keys);
    value["deployed"] = serde_json::Value::Bool(false);
    value["databases"] = serde_json::to_value(databases).map_err(anyhow::Error::from)?;
    value["redis"] = serde_json::to_value(instance).map_err(anyhow::Error::from)?;
    value["managed"] = serde_json::to_value(managed).map_err(anyhow::Error::from)?;
    Ok(Json(value))
}

async fn find(app: &AppState, slug: &str) -> ApiResult<apps::App> {
    apps::by_slug(&app.db, slug)
        .await?
        .ok_or_else(|| ApiError::not_found(AppError::NotFound.to_string()))
}

async fn link_database(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path((slug, name)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let found = find(&app, &slug).await?;
    postgres::link(&app.db, &found.id, &name)
        .await
        .map_err(crate::routes::databases::db_error)?;
    provision::write_env(&app.db, app.platform.as_ref(), &found).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unlink_database(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path((slug, name)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    let found = find(&app, &slug).await?;
    let done = postgres::unlink(&app.db, &found.id, &name)
        .await
        .map_err(crate::routes::databases::db_error)?;
    if !done {
        return Err(ApiError::not_found(format!(
            "{name} is not linked to {slug}."
        )));
    }
    provision::write_env(&app.db, app.platform.as_ref(), &found).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn request_redis(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
    body: String,
) -> ApiResult<(StatusCode, Json<redis::Instance>)> {
    let found = find(&app, &slug).await?;
    let request: RedisRequest = if body.trim().is_empty() {
        RedisRequest::default()
    } else {
        serde_json::from_str(&body).map_err(|e| ApiError::bad_request(e.to_string()))?
    };
    let maxmemory_mb = request.maxmemory_mb.unwrap_or(redis::DEFAULT_MAXMEMORY_MB);
    redis::ensure_installed(app.platform.as_ref(), &app.codename)
        .map_err(|e| ApiError::bad_request(format!("Installing Redis failed: {e}")))?;
    let instance = redis::request(&app.db, app.platform.as_ref(), &found, maxmemory_mb)
        .await
        .map_err(|e| match e.downcast_ref::<RedisError>() {
            Some(RedisError::Exists(_)) => ApiError::conflict(e.to_string()),
            Some(RedisError::Invalid) => ApiError::bad_request(e.to_string()),
            None => ApiError::bad_request(format!("The host could not start Redis: {e:#}")),
        })?;
    provision::write_env(&app.db, app.platform.as_ref(), &found).await?;
    Ok((StatusCode::CREATED, Json(instance)))
}

async fn release_redis(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let found = find(&app, &slug).await?;
    if !redis::release(&app.db, app.platform.as_ref(), &found).await? {
        return Err(ApiError::not_found(format!(
            "{slug} has no Redis instance."
        )));
    }
    provision::write_env(&app.db, app.platform.as_ref(), &found).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn update(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
    Json(changes): Json<AppChanges>,
) -> ApiResult<Json<apps::App>> {
    if let Some(packages) = &changes.packages {
        let resolved: Vec<String> = packages
            .iter()
            .filter(|p| detect::valid_package(p))
            .flat_map(|p| app.platform.resolve_package(p))
            .collect();
        if !resolved.is_empty() {
            let names: Vec<&str> = resolved.iter().map(String::as_str).collect();
            app.platform
                .install_packages(&names)
                .map_err(|e| ApiError::bad_request(format!("Installing packages failed: {e}")))?;
        }
    }
    let updated = apps::update(&app.db, &slug, changes)
        .await
        .map_err(app_error)?;
    provision::reprovision(&app.db, app.platform.as_ref(), &updated)
        .await
        .map_err(|e| ApiError::bad_request(format!("The host refused the change: {e:#}")))?;
    Ok(Json(updated))
}

async fn remove(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
    Json(body): Json<Removal>,
) -> ApiResult<StatusCode> {
    let found = apps::by_slug(&app.db, &slug)
        .await?
        .ok_or_else(|| ApiError::not_found(AppError::NotFound.to_string()))?;
    if body.name.trim() != found.name {
        return Err(ApiError::bad_request(
            "Type the application's name exactly to delete it.",
        ));
    }
    provision::deprovision(&app.db, app.platform.as_ref(), &found).await?;
    apps::delete(&app.db, &slug).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_env(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
    Json(vars): Json<Vec<env::EnvChange>>,
) -> ApiResult<StatusCode> {
    let found = apps::by_slug(&app.db, &slug)
        .await?
        .ok_or_else(|| ApiError::not_found(AppError::NotFound.to_string()))?;
    env::replace(&app.db, &found.id, &vars)
        .await
        .map_err(app_error)?;
    provision::write_env(&app.db, app.platform.as_ref(), &found).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn app_error(e: anyhow::Error) -> ApiError {
    match e.downcast_ref::<AppError>() {
        Some(AppError::SlugTaken(_)) => ApiError::conflict(e.to_string()),
        Some(AppError::NotFound) => ApiError::not_found(e.to_string()),
        Some(AppError::Invalid(_)) | Some(AppError::NoProcess) => {
            ApiError::bad_request(e.to_string())
        }
        None => e.into(),
    }
}
