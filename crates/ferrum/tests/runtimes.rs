mod support;

use axum::http::StatusCode;
use ferrum_core::runtime::RuntimeKind;
use support::{signed_in, signed_in_and_connected, signed_in_with_downloads};

#[tokio::test]
async fn installing_a_runtime_streams_progress_and_ends_with_ready() {
    let (h, cookie, downloads) = signed_in_with_downloads().await;
    let res = h
        .post_with_cookie("/api/runtimes/node/22.11.0", "", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert!(
        res.header("content-type")
            .unwrap()
            .starts_with("text/event-stream"),
        "{:?}",
        res.header("content-type")
    );
    assert!(
        res.text.contains(r#""state":"downloading""#),
        "{}",
        res.text
    );
    assert!(res.text.contains(r#""state":"extracting""#), "{}", res.text);
    assert!(
        res.text.trim_end().ends_with(r#"data: {"state":"ready"}"#),
        "{}",
        res.text
    );
    assert_eq!(downloads.hits(), 1);
    assert!(
        h.toolchains
            .dir(RuntimeKind::Node, "22.11.0")
            .join("bin/node")
            .exists()
    );

    let listed = h.get_with_cookie("/api/runtimes", &cookie).await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(listed.json["installed"][0]["kind"], "node");
    assert_eq!(listed.json["installed"][0]["version"], "22.11.0");
    assert!(listed.json["installed"][0]["size_bytes"].as_u64().unwrap() > 0);
    assert_eq!(listed.json["dotnet_channels"][0], "10.0");

    let again = h
        .post_with_cookie("/api/runtimes/node/22.11.0", "", &cookie)
        .await;
    assert!(
        again
            .text
            .trim_end()
            .ends_with(r#"data: {"state":"ready"}"#)
    );
    assert_eq!(
        downloads.hits(),
        1,
        "an installed toolchain is not fetched again"
    );
}

#[tokio::test]
async fn a_failed_install_streams_the_failure_and_records_nothing() {
    let (h, cookie, downloads) = signed_in_with_downloads().await;
    h.platform.fail_next("extract_tar_gz");
    let res = h
        .post_with_cookie("/api/runtimes/node/22.11.0", "", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.text.contains(r#""state":"failed""#), "{}", res.text);
    assert_eq!(downloads.hits(), 1);
    let listed = h.get_with_cookie("/api/runtimes", &cookie).await;
    assert!(listed.json["installed"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_partial_version_or_unknown_runtime_is_refused_up_front() {
    let (h, cookie, downloads) = signed_in_with_downloads().await;
    assert_eq!(
        h.post_with_cookie("/api/runtimes/node/22", "", &cookie)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.post_with_cookie("/api/runtimes/static/1.0.0", "", &cookie)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        h.post_with_cookie("/api/runtimes/python/3.12.0", "", &cookie)
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(downloads.hits(), 0);
}

#[tokio::test]
async fn versions_resolve_from_each_vendor() {
    let (h, cookie, _downloads) = signed_in_with_downloads().await;
    let node = h
        .get_with_cookie("/api/runtimes/node/resolve?version=22", &cookie)
        .await;
    assert_eq!(node.status, StatusCode::OK, "{}", node.json);
    assert_eq!(node.json["version"], "22.11.0");
    let lts = h
        .get_with_cookie("/api/runtimes/node/resolve", &cookie)
        .await;
    assert_eq!(lts.json["version"], "24.9.0");

    let dotnet = h
        .get_with_cookie("/api/runtimes/dotnet/resolve?version=9.0", &cookie)
        .await;
    assert_eq!(dotnet.json["version"], "9.0");
    let dotnet_default = h
        .get_with_cookie("/api/runtimes/dotnet/resolve", &cookie)
        .await;
    assert_eq!(dotnet_default.json["version"], "10.0");

    let (h, cookie, _github) = signed_in_and_connected().await;
    let bun = h
        .get_with_cookie("/api/runtimes/bun/resolve", &cookie)
        .await;
    assert_eq!(bun.status, StatusCode::OK, "{}", bun.json);
    assert_eq!(bun.json["version"], support::github_stub::BUN_LATEST);
    let pinned = h
        .get_with_cookie("/api/runtimes/bun/resolve?version=1.0.0", &cookie)
        .await;
    assert_eq!(pinned.json["version"], "1.0.0");
}

#[tokio::test]
async fn a_read_only_token_can_see_runtimes_but_not_install_them() {
    let (h, _cookie) = signed_in().await;
    let token = h.machine_token(true).await;
    assert_eq!(
        h.get_with_bearer("/api/runtimes", &token).await.status,
        StatusCode::OK
    );
    assert_eq!(
        h.post_with_bearer("/api/runtimes/node/22.11.0", "", &token)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}
