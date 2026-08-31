use crate::auth::Caller;
use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct Me {
    pub kind: &'static str,
    pub name: String,
    pub read_only: bool,
}

pub async fn get(caller: Caller) -> Json<Me> {
    Json(Me {
        kind: caller.kind(),
        name: caller.name().to_string(),
        read_only: caller.is_read_only(),
    })
}
