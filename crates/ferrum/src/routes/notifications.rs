use crate::auth::Caller;
use crate::routes::error::{ApiError, ApiResult};
use crate::server::AppState;
use axum::extract::{Query, State as Extract};
use axum::http::StatusCode;
use axum::{Json, Router, routing::get, routing::post};
use ferrum_core::events::{self, Event, PREFS};
use ferrum_core::push::{self, Device, InvalidSubscription, Message, Outcome, Subscription, Vapid};
use ferrum_core::time;
use ferrum_core::users::User;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

const MACHINE: &str = "Notifications belong to a person; an API token has none.";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/push/vapid", get(vapid))
        .route(
            "/api/push/devices",
            get(devices).post(register).delete(unregister),
        )
        .route("/api/push/prefs", get(prefs).put(set_prefs))
        .route("/api/push/test", post(test))
        .route("/api/events", get(list))
        .route("/api/events/read", post(read))
        .route("/api/events/unread", get(unread))
}

fn person(caller: &Caller) -> ApiResult<&User> {
    caller
        .user()
        .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, MACHINE))
}

#[derive(Serialize)]
struct Key {
    key: String,
}

async fn vapid(Extract(app): Extract<AppState>, caller: Caller) -> ApiResult<Json<Key>> {
    person(&caller)?;
    Ok(Json(Key {
        key: push::public_key(&app.db).await?,
    }))
}

async fn devices(Extract(app): Extract<AppState>, caller: Caller) -> ApiResult<Json<Vec<Device>>> {
    let user = person(&caller)?;
    Ok(Json(push::list_for(&app.db, &user.id).await?))
}

async fn register(
    Extract(app): Extract<AppState>,
    caller: Caller,
    Json(sub): Json<Subscription>,
) -> ApiResult<(StatusCode, Json<Device>)> {
    let user = person(&caller)?;
    let device = push::register(&app.db, &user.id, &sub).await.map_err(|e| {
        match e.downcast_ref::<InvalidSubscription>() {
            Some(invalid) => ApiError::bad_request(invalid.to_string()),
            None => e.into(),
        }
    })?;
    Ok((StatusCode::CREATED, Json(device)))
}

#[derive(Deserialize)]
struct Endpoint {
    endpoint: String,
}

async fn unregister(
    Extract(app): Extract<AppState>,
    caller: Caller,
    Json(body): Json<Endpoint>,
) -> ApiResult<StatusCode> {
    let user = person(&caller)?;
    push::unregister(&app.db, &user.id, &body.endpoint).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct Prefs {
    enabled: BTreeMap<&'static str, bool>,
}

async fn prefs(Extract(app): Extract<AppState>, caller: Caller) -> ApiResult<Json<Prefs>> {
    let user = person(&caller)?;
    Ok(Json(Prefs {
        enabled: push::get_prefs(&app.db, &user.id).await?,
    }))
}

async fn set_prefs(
    Extract(app): Extract<AppState>,
    caller: Caller,
    Json(changes): Json<HashMap<String, bool>>,
) -> ApiResult<Json<Prefs>> {
    let user = person(&caller)?;
    if let Some(unknown) = changes.keys().find(|k| !PREFS.contains(&k.as_str())) {
        return Err(ApiError::bad_request(format!(
            "{unknown} is not a notification kind."
        )));
    }
    for (kind, enabled) in &changes {
        push::set_pref(&app.db, &user.id, kind, *enabled).await?;
    }
    prefs(Extract(app), caller).await
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Delivery {
    Delivered,
    Gone,
    Rejected,
    Failed,
}

#[derive(Serialize)]
struct Tried {
    id: String,
    result: Delivery,
    status: Option<u16>,
}

async fn test(Extract(app): Extract<AppState>, caller: Caller) -> ApiResult<Json<Vec<Tried>>> {
    let user = person(&caller)?;
    let devices = push::list_for(&app.db, &user.id).await?;
    if devices.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let vapid = Vapid::load(&app.db)
        .await
        .map_err(|e| ApiError::conflict(e.to_string()))?;
    let message = Message {
        title: "Test notification",
        body: "Notifications from this server reach this device.",
        link: "/settings?tab=about",
    };
    let mut tried = Vec::with_capacity(devices.len());
    for device in devices {
        let (result, status) =
            match push::deliver(&app.db, &app.http, &vapid, &device, &message).await {
                Ok(Outcome::Delivered) => (Delivery::Delivered, None),
                Ok(Outcome::Gone) => (Delivery::Gone, None),
                Ok(Outcome::Rejected(status)) => (Delivery::Rejected, Some(status)),
                Err(error) => {
                    tracing::warn!(device = %device.id, error = ?error, "test push not sent");
                    (Delivery::Failed, None)
                }
            };
        tried.push(Tried {
            id: device.id,
            result,
            status,
        });
    }
    Ok(Json(tried))
}

#[derive(Deserialize)]
struct Listing {
    unread: Option<String>,
    limit: Option<i64>,
}

async fn list(
    Extract(app): Extract<AppState>,
    _: Caller,
    Query(q): Query<Listing>,
) -> ApiResult<Json<Vec<Event>>> {
    let unread = matches!(q.unread.as_deref(), Some("1" | "true"));
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let events = events::list(&app.db, limit, unread)
        .await?
        .into_iter()
        .map(|e| Event {
            created_at: time::utc(e.created_at),
            read_at: time::utc_opt(e.read_at),
            ..e
        })
        .collect();
    Ok(Json(events))
}

#[derive(Deserialize)]
struct Read {
    #[serde(default)]
    ids: Vec<String>,
    #[serde(default)]
    all: bool,
}

async fn read(
    Extract(app): Extract<AppState>,
    _: Caller,
    Json(body): Json<Read>,
) -> ApiResult<StatusCode> {
    if body.all {
        events::mark_all_read(&app.db).await?;
    } else {
        events::mark_read(&app.db, &body.ids).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct Count {
    count: i64,
}

async fn unread(Extract(app): Extract<AppState>, _: Caller) -> ApiResult<Json<Count>> {
    Ok(Json(Count {
        count: events::unread_count(&app.db).await?,
    }))
}
