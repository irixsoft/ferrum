use crate::server::AppState;
use axum::Router;

pub fn router() -> Router<AppState> {
    Router::new()
}
