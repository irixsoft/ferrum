use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct Health {
    pub ok: bool,
}

pub async fn get() -> Json<Health> {
    Json(Health { ok: true })
}
