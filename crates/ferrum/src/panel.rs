use crate::routes::error::ApiError;
use crate::server::AppState;
use axum::Router;
use axum::body::Body;
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;
use std::borrow::Cow;

#[derive(RustEmbed)]
#[folder = "../../web/dist/"]
#[allow_missing = true]
struct Panel;

const INDEX: &str = "index.html";
const HASHED: &str = "assets/";
const IMMUTABLE: &str = "public, max-age=31536000, immutable";
const REVALIDATE: &str = "no-cache";

pub fn router() -> Router<AppState> {
    Router::new().fallback(serve)
}

async fn serve(uri: Uri) -> Response {
    let path = uri.path();
    if path == "/api" || path.starts_with("/api/") || path.starts_with("/mcp") {
        return ApiError::not_found("No such endpoint.").into_response();
    }

    file(path.trim_start_matches('/'))
        .or_else(|| file(INDEX))
        .unwrap_or_else(unbuilt)
}

fn file(path: &str) -> Option<Response> {
    let asset = Panel::get(path)?;
    let body = match asset.data {
        Cow::Borrowed(bytes) => Body::from(bytes),
        Cow::Owned(bytes) => Body::from(bytes),
    };

    let cache = if path.starts_with(HASHED) {
        IMMUTABLE
    } else {
        REVALIDATE
    };

    Some(
        (
            [
                (header::CONTENT_TYPE, content_type(path)),
                (header::CACHE_CONTROL, cache.to_string()),
            ],
            body,
        )
            .into_response(),
    )
}

fn content_type(path: &str) -> String {
    if path.ends_with(".webmanifest") {
        return "application/manifest+json".to_string();
    }
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string()
}

fn unbuilt() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "The panel was not built into this binary.",
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashed_assets_are_immutable_and_the_entry_points_are_not() {
        assert!(!INDEX.starts_with(HASHED));
        for entry in ["sw.js", "manifest.webmanifest", "registerSW.js"] {
            assert!(
                !entry.starts_with(HASHED),
                "{entry} must be revalidated, or a self-update serves the old panel forever"
            );
        }
    }

    #[test]
    fn the_manifest_gets_its_own_content_type() {
        assert_eq!(
            content_type("manifest.webmanifest"),
            "application/manifest+json"
        );
        assert!(content_type("index.html").starts_with("text/html"));
        assert!(content_type("assets/index.css").starts_with("text/css"));
        assert!(content_type("assets/index.js").contains("javascript"));
    }
}
