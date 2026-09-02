mod support;

use axum::http::StatusCode;
use support::harness;

#[tokio::test]
async fn the_index_is_served_at_the_root() {
    let h = harness().await;
    let res = h.get("/").await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert!(res.text.contains("<div id=\"root\">"), "{}", res.text);
    assert!(res.header("content-type").unwrap().starts_with("text/html"));
}

#[tokio::test]
async fn a_client_route_falls_back_to_the_index() {
    let h = harness().await;
    for route in ["/apps/my-app", "/settings", "/enroll/abc123"] {
        let res = h.get(route).await;
        assert_eq!(res.status, StatusCode::OK, "{route}");
        assert!(res.text.contains("<div id=\"root\">"), "{route}");
    }
}

#[tokio::test]
async fn an_unknown_api_path_is_json_404_not_the_index() {
    let h = harness().await;
    for (route, status) in [
        ("/api/nope", StatusCode::NOT_FOUND),
        ("/api/users/1/nope", StatusCode::NOT_FOUND),
        ("/mcp", StatusCode::UNAUTHORIZED),
    ] {
        let res = h.get(route).await;
        assert_eq!(res.status, status, "{route}");
        assert!(
            !res.text.contains("<div id=\"root\">"),
            "the panel must never answer for {route}: {}",
            res.text
        );
        assert!(res.json["error"].is_string(), "{route}: {}", res.json);
    }
}

#[tokio::test]
async fn a_hashed_asset_is_served_immutably_with_its_own_content_type() {
    let h = harness().await;
    let index = h.get("/").await.text;
    let css = hashed(&index, ".css");

    let res = h.get(&css).await;
    assert_eq!(res.status, StatusCode::OK, "{css}");
    assert!(res.header("content-type").unwrap().starts_with("text/css"));
    assert_eq!(
        res.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
}

#[tokio::test]
async fn the_entry_points_are_never_cached() {
    let h = harness().await;
    for route in ["/", "/sw.js", "/manifest.webmanifest"] {
        let res = h.get(route).await;
        assert_eq!(res.status, StatusCode::OK, "{route}");
        assert_eq!(
            res.header("cache-control"),
            Some("no-cache"),
            "a cached {route} serves the previous version of the panel after an update"
        );
    }
}

#[tokio::test]
async fn the_manifest_is_served_as_a_manifest() {
    let h = harness().await;
    let res = h.get("/manifest.webmanifest").await;
    assert_eq!(
        res.header("content-type"),
        Some("application/manifest+json")
    );
    assert_eq!(res.json["name"], "Ferrum");
}

#[tokio::test]
async fn the_panel_is_reachable_without_signing_in() {
    let h = harness().await;
    assert_eq!(h.get("/").await.status, StatusCode::OK);
    assert_eq!(
        h.get("/api/me").await.status,
        StatusCode::UNAUTHORIZED,
        "serving the shell must not serve the data behind it"
    );
}

fn hashed(index: &str, extension: &str) -> String {
    index
        .split('"')
        .find(|part| part.starts_with("/assets/") && part.ends_with(extension))
        .unwrap_or_else(|| panic!("no {extension} in the built index; run `bun run build` in web/"))
        .to_string()
}
