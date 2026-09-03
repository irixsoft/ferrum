use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Path, State as Extract};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get};
use ferrum_core::apps::unit::unit_name;
use ferrum_core::apps::{self, App, AppChanges, AppError, NewApp, env, provision};
use ferrum_core::deploy::{self, Outcome, maintenance, releases};
use ferrum_core::detect::{self, DetectError, Detected};
use ferrum_core::redis::{self, RedisError};
use ferrum_core::runtime::toolchain;
use ferrum_core::{certs, github, metrics, postgres};
use ferrum_platform::ServiceAction;
use serde::{Deserialize, Serialize};

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
        .route(
            "/api/apps/{slug}/certificates",
            axum::routing::post(retry_certificates),
        )
        .route("/api/apps/{slug}/restart", axum::routing::post(restart))
}

const RESTART_WHILE_DEPLOYING: &str =
    "A deploy is running for this application; it restarts by itself when the deploy ends.";

#[derive(Serialize)]
pub(crate) struct Listed {
    #[serde(flatten)]
    app: App,
    status: &'static str,
    never_live: bool,
}

/// What the panel's pill says: the running deploy first, then the maintenance flag, then the
/// unit.
async fn status_of(state: &AppState, app: &App) -> anyhow::Result<&'static str> {
    if deploy::running_for(&state.db, &app.id).await?.is_some() {
        return Ok("building");
    }
    if maintenance::is_on(state.platform.as_ref(), &app.slug) {
        return Ok("maintenance");
    }
    if app.current_release_id.is_none() {
        let failed = deploy::latest_for(&state.db, &app.id)
            .await?
            .is_some_and(|d| d.outcome == Some(Outcome::Failed));
        return Ok(if failed { "failed" } else { "new" });
    }
    if !app.runtime.has_process() || state.platform.service_is_active(&unit_name(&app.slug)) {
        Ok("live")
    } else {
        Ok("stopped")
    }
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

pub(crate) async fn listed(app: &AppState) -> anyhow::Result<Vec<Listed>> {
    let mut listed = Vec::new();
    for found in apps::list(&app.db).await? {
        let status = status_of(app, &found).await?;
        let never_live = found.current_release_id.is_none();
        listed.push(Listed {
            app: found,
            status,
            never_live,
        });
    }
    Ok(listed)
}

async fn list(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Vec<Listed>>> {
    Ok(Json(listed(&app).await?))
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
    app.issue_certificates_later(created.clone());
    Ok((StatusCode::CREATED, Json(created)))
}

async fn show(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let found = find(&app, &slug).await?;
    Ok(Json(detail(&app, &found).await?))
}

/// Everything the panel's app page reads: env *keys*, never values.
pub(crate) async fn detail(app: &AppState, found: &App) -> anyhow::Result<serde_json::Value> {
    let keys: Vec<serde_json::Value> = env::keys(&app.db, &found.id)
        .await?
        .into_iter()
        .map(|key| serde_json::json!({ "key": key }))
        .collect();
    let databases = postgres::names_for(&app.db, &found.id).await?;
    let instance = redis::for_app(&app.db, &found.id).await?;
    let managed = env::managed_for(&app.db, found).await?.keys();
    let current = match &found.current_release_id {
        Some(id) => releases::by_id(&app.db, id).await?,
        None => None,
    };
    let status = status_of(app, found).await?;
    let certificates = certs::statuses(&app.db, app.platform.as_ref(), found).await?;
    let resources = if found.runtime.has_process() {
        app.platform.cgroup_stats(&unit_name(&found.slug))?
    } else {
        None
    };
    let cpu_pct = match resources {
        Some(_) => Some(
            metrics::latest(&app.db, &found.id)
                .await?
                .map(|s| s.cpu_pct)
                .unwrap_or(0.0),
        ),
        None => None,
    };
    let mut value = serde_json::to_value(found)?;
    value["memory_bytes"] = resources.map(|s| s.memory_current).into();
    value["memory_peak_bytes"] = resources.map(|s| s.memory_peak).into();
    value["cpu_pct"] = cpu_pct.into();
    value["env"] = serde_json::Value::Array(keys);
    value["deployed"] = serde_json::Value::Bool(current.is_some());
    value["current_release"] = serde_json::to_value(current)?;
    value["status"] = serde_json::Value::String(status.into());
    value["never_live"] = serde_json::Value::Bool(found.current_release_id.is_none());
    value["certificates"] = serde_json::to_value(certificates)?;
    value["databases"] = serde_json::to_value(databases)?;
    value["redis"] = serde_json::to_value(instance)?;
    value["managed"] = serde_json::to_value(managed)?;
    Ok(value)
}

async fn retry_certificates(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let found = find(&app, &slug).await?;
    if found.domains.is_empty() {
        return Err(ApiError::bad_request(
            "Add a domain under Configuration first.",
        ));
    }
    certs::retry_now(&app.db, &found).await?;
    app.issue_certificates_later(found);
    Ok(StatusCode::ACCEPTED)
}

async fn restart(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(slug): Path<String>,
) -> ApiResult<StatusCode> {
    let found = find(&app, &slug).await?;
    restart_unit(&app, &found).await?;
    Ok(StatusCode::ACCEPTED)
}

pub(crate) async fn restart_unit(app: &AppState, found: &App) -> ApiResult<()> {
    if !found.runtime.has_process() {
        return Err(ApiError::bad_request(
            "A static site has no process to restart.",
        ));
    }
    if deploy::running_for(&app.db, &found.id).await?.is_some() {
        return Err(ApiError::conflict(RESTART_WHILE_DEPLOYING));
    }
    if found.current_release_id.is_none() {
        return Err(ApiError::conflict(format!(
            "{} has not been deployed yet; there is nothing to restart.",
            found.slug
        )));
    }
    app.platform
        .service(ServiceAction::Restart, &unit_name(&found.slug))
        .map_err(|e| ApiError::bad_request(format!("The host refused the restart: {e}")))?;
    Ok(())
}

pub(crate) async fn find(app: &AppState, slug: &str) -> ApiResult<apps::App> {
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
    Ok(Json(apply(&app, &slug, changes).await?))
}

/// Packages first, then the row, then the host; certificates follow in the background.
pub(crate) async fn apply(app: &AppState, slug: &str, changes: AppChanges) -> ApiResult<App> {
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
    let updated = apps::update(&app.db, slug, changes)
        .await
        .map_err(app_error)?;
    provision::reprovision(&app.db, app.platform.as_ref(), &updated)
        .await
        .map_err(|e| ApiError::bad_request(format!("The host refused the change: {e:#}")))?;
    app.issue_certificates_later(updated.clone());
    Ok(updated)
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
    let found = find(&app, &slug).await?;
    replace_env(&app, &found, &vars).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn replace_env(
    app: &AppState,
    found: &App,
    vars: &[env::EnvChange],
) -> ApiResult<()> {
    env::replace(&app.db, &found.id, vars)
        .await
        .map_err(app_error)?;
    provision::write_env(&app.db, app.platform.as_ref(), found).await?;
    Ok(())
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
