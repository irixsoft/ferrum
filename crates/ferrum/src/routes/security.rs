use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::{AppState, Install, Job, JobStatus};
use axum::extract::{Path, State as Extract};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get, routing::post};
use ferrum_core::security::{self, Security, SecurityError, bans, firewall, ssh, updates};
use ferrum_core::setup;
use ferrum_platform::PlatformError;
use serde::{Deserialize, Serialize};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/security", get(status))
        .route("/api/security/firewall", post(enable_firewall))
        .route("/api/security/fail2ban", post(enable_fail2ban))
        .route("/api/security/updates", post(enable_updates))
        .route("/api/security/bans/{ip}/unban", post(unban))
        .route("/api/security/allowlist", post(allowlist))
        .route(
            "/api/security/ssh/disable-passwords",
            post(disable_passwords),
        )
}

#[derive(Serialize)]
pub(crate) struct View {
    #[serde(flatten)]
    security: Security,
    jobs: Jobs,
}

#[derive(Serialize)]
struct Jobs {
    firewall: JobStatus,
    fail2ban: JobStatus,
    updates: JobStatus,
}

pub(crate) async fn view(app: &AppState) -> ApiResult<View> {
    let security = security::status(&app.db, app.platform.as_ref())
        .await
        .map_err(security_error)?;
    let jobs = app.hardening.lock().unwrap();
    Ok(View {
        security,
        jobs: Jobs {
            firewall: jobs.get(&Job::Firewall).into(),
            fail2ban: jobs.get(&Job::Fail2ban).into(),
            updates: jobs.get(&Job::Updates).into(),
        },
    })
}

async fn status(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<View>> {
    Ok(Json(view(&app).await?))
}

/// apt takes a minute, so each enable claims its slot, runs on a blocking thread and answers at once.
fn start(
    app: &AppState,
    job: Job,
    what: &str,
    work: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
) -> ApiResult<()> {
    {
        let mut jobs = app.hardening.lock().unwrap();
        if jobs.get(&job) == Some(&Install::Running) {
            return Err(ApiError::conflict(format!(
                "{what} is already being enabled."
            )));
        }
        jobs.insert(job, Install::Running);
    }
    let slots = app.hardening.clone();
    tokio::spawn(async move {
        let outcome = match tokio::task::spawn_blocking(work).await {
            Ok(Ok(())) => Install::Idle,
            Ok(Err(e)) => Install::Failed(security_error(e).message),
            Err(e) => Install::Failed(e.to_string()),
        };
        slots.lock().unwrap().insert(job, outcome);
    });
    Ok(())
}

async fn accepted(app: &AppState) -> ApiResult<(StatusCode, Json<View>)> {
    Ok((StatusCode::ACCEPTED, Json(view(app).await?)))
}

async fn enable_firewall(
    Extract(app): Extract<AppState>,
    _: Caller,
) -> ApiResult<(StatusCode, Json<View>)> {
    let enabled = app
        .platform
        .ufw_status()
        .map_err(|e| security_error(e.into()))?;
    if enabled.is_some() {
        return Err(ApiError::conflict(
            SecurityError::AlreadyEnabled.to_string(),
        ));
    }
    let platform = app.platform.clone();
    start(&app, Job::Firewall, "The firewall", move || {
        firewall::enable(platform.as_ref())
    })?;
    accepted(&app).await
}

async fn enable_fail2ban(
    Extract(app): Extract<AppState>,
    _: Caller,
) -> ApiResult<(StatusCode, Json<View>)> {
    let allowlist = bans::allowlist(&app.db).await?;
    let platform = app.platform.clone();
    start(&app, Job::Fail2ban, "fail2ban", move || {
        bans::enable(platform.as_ref(), &allowlist)
    })?;
    accepted(&app).await
}

async fn enable_updates(
    Extract(app): Extract<AppState>,
    _: Caller,
) -> ApiResult<(StatusCode, Json<View>)> {
    let platform = app.platform.clone();
    start(&app, Job::Updates, "Security updates", move || {
        updates::enable(platform.as_ref())
    })?;
    accepted(&app).await
}

async fn unban(
    Extract(app): Extract<AppState>,
    _: Caller,
    Path(ip): Path<String>,
) -> ApiResult<StatusCode> {
    bans::unban(app.platform.as_ref(), &ip).map_err(security_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Address {
    ip: String,
}

async fn allowlist(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Address>,
) -> ApiResult<StatusCode> {
    bans::allow(&app.db, app.platform.as_ref(), &body.ip)
        .await
        .map_err(security_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Confirmation {
    name: String,
}

/// Typing the hostname is the confirmation; the keys are checked after, by the core.
async fn disable_passwords(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Confirmation>,
) -> ApiResult<StatusCode> {
    let hostname = setup::hostname(&app.db).await?.unwrap_or_default();
    if body.name.trim() != hostname {
        return Err(ApiError::bad_request(format!(
            "Type the hostname, {hostname}, to confirm."
        )));
    }
    ssh::disable_passwords(app.platform.as_ref()).map_err(security_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn security_error(e: anyhow::Error) -> ApiError {
    match e.downcast_ref::<SecurityError>() {
        Some(SecurityError::AlreadyEnabled) => ApiError::conflict(e.to_string()),
        Some(SecurityError::NotBanned(_)) => ApiError::not_found(e.to_string()),
        Some(_) => ApiError::bad_request(e.to_string()),
        None => match e.downcast_ref::<PlatformError>() {
            Some(refused) => ApiError::bad_request(format!("The host refused: {refused}")),
            None => e.into(),
        },
    }
}
