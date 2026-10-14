use crate::auth::Caller;
use axum::Json;
use serde::Serialize;

#[derive(Serialize)]
pub struct Me {
    pub kind: &'static str,
    pub id: Option<String>,
    pub name: String,
    pub read_only: bool,
}

pub async fn get(caller: Caller) -> Json<Me> {
    Json(Me {
        kind: caller.kind(),
        id: caller.user().map(|u| u.id.clone()),
        name: caller.name().to_string(),
        read_only: caller.is_read_only(),
    })
}
