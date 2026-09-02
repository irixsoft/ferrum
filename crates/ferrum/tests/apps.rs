mod support;

use axum::http::StatusCode;
use ferrum_core::runtime::RuntimeKind;
use support::{new_app_json, signed_in, signed_in_and_connected};

const NEXT_PACKAGE: &str = r#"{"scripts":{"build":"next build","start":"next start"}}"#;

#[tokio::test]
async fn detection_reads_the_tree_and_the_files_it_needs_and_nothing_else() {
    let (h, cookie, github) = signed_in_and_connected().await;
    github.serve_repo(
        "irixsoft/ledger",
        &[
            ("package.json", NEXT_PACKAGE),
            ("next.config.js", ""),
            ("bun.lockb", ""),
            ("README.md", "# no"),
            ("src/app/page.tsx", "export default () => null"),
        ],
    );

    let res = h
        .post_with_cookie(
            "/api/apps/detect",
            r#"{"repository":"irixsoft/ledger","ref":"main"}"#,
            &cookie,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["candidates"][0]["kind"], "node");
    assert_eq!(
        res.json["candidates"][0]["commands"]["install"],
        "bun install --frozen-lockfile"
    );
    assert!(
        res.json["candidates"][0]["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r.as_str().unwrap().contains("next.config.js"))
    );
    assert_eq!(
        github.contents_fetched(),
        vec!["package.json"],
        "README.md must not be fetched"
    );
}

#[tokio::test]
async fn a_root_directory_scopes_detection_to_a_subfolder() {
    let (h, cookie, github) = signed_in_and_connected().await;
    github.serve_repo(
        "irixsoft/ledger",
        &[
            ("apps/web/package.json", NEXT_PACKAGE),
            ("apps/web/next.config.js", ""),
            (
                "apps/api/Api.csproj",
                r#"<Project Sdk="Microsoft.NET.Sdk.Web">"#,
            ),
            ("Aptfile", "ffmpeg\n"),
        ],
    );
    let res = h
        .post_with_cookie(
            "/api/apps/detect",
            r#"{"repository":"irixsoft/ledger","ref":"main","root":"apps/web"}"#,
            &cookie,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    let kinds: Vec<&str> = res.json["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec!["node"],
        "the csproj outside the root is not seen"
    );
    assert_eq!(github.contents_fetched(), vec!["apps/web/package.json"]);
    assert!(
        res.json["aptfile"].as_array().unwrap().is_empty(),
        "the Aptfile outside the root is not read"
    );
}

#[tokio::test]
async fn a_truncated_tree_is_reported_rather_than_read_as_empty() {
    let (h, cookie, github) = signed_in_and_connected().await;
    github.serve_truncated_tree("irixsoft/huge");
    let res = h
        .post_with_cookie(
            "/api/apps/detect",
            r#"{"repository":"irixsoft/huge","ref":"main"}"#,
            &cookie,
        )
        .await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY, "{}", res.json);
    assert!(res.json["error"].as_str().unwrap().contains("too large"));
}

#[tokio::test]
async fn an_unknown_ref_is_a_404_with_the_ref_named() {
    let (h, cookie, github) = signed_in_and_connected().await;
    github.serve_repo("irixsoft/ledger", &[("package.json", "{}")]);
    let res = h
        .post_with_cookie(
            "/api/apps/detect",
            r#"{"repository":"irixsoft/ledger","ref":"missing"}"#,
            &cookie,
        )
        .await;
    assert_eq!(res.status, StatusCode::NOT_FOUND, "{}", res.json);
    assert!(res.json["error"].as_str().unwrap().contains("missing"));
}

#[tokio::test]
async fn detection_without_github_says_so() {
    let (h, cookie) = signed_in().await;
    let res = h
        .post_with_cookie(
            "/api/apps/detect",
            r#"{"repository":"irixsoft/ledger","ref":"main"}"#,
            &cookie,
        )
        .await;
    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE, "{}", res.json);
}

#[tokio::test]
async fn creating_an_app_provisions_it_and_returns_never_deployed() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.pretend_toolchain(RuntimeKind::Node, "22.11.0").await;
    let res = h
        .post_with_cookie("/api/apps", &new_app_json("ledger"), &cookie)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["slug"], "ledger");
    assert!(res.json["routes"][0]["port"].as_u64().unwrap() >= 20000);
    assert!(
        h.platform
            .calls()
            .iter()
            .any(|c| c.starts_with("create_system_user ferrum-ledger"))
    );

    let got = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert_eq!(got.status, StatusCode::OK);
    assert_eq!(got.json["deployed"], false);
    assert_eq!(got.json["env"], serde_json::json!([]));

    let listed = h.get_with_cookie("/api/apps", &cookie).await;
    assert_eq!(listed.json.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn creating_an_app_needs_its_toolchain_installed_first() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    let res = h
        .post_with_cookie("/api/apps", &new_app_json("ledger"), &cookie)
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.json);
    assert!(res.json["error"].as_str().unwrap().contains("22.11.0"));
    assert!(h.platform.calls().is_empty(), "nothing touches the host");
}

#[tokio::test]
async fn env_values_are_write_only() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.pretend_toolchain(RuntimeKind::Node, "22.11.0").await;
    h.post_with_cookie("/api/apps", &new_app_json("ledger"), &cookie)
        .await;
    let res = h
        .put_with_cookie(
            "/api/apps/ledger/env",
            r#"[{"key":"SECRET","value":"hunter2"}]"#,
            &cookie,
        )
        .await;
    assert_eq!(res.status, StatusCode::NO_CONTENT, "{}", res.json);

    let got = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert_eq!(got.json["env"][0]["key"], "SECRET");
    assert!(!got.text.contains("hunter2"), "{}", got.text);
    let env = h
        .platform
        .written("/var/lib/ferrum/apps/ledger/shared/.env")
        .unwrap();
    assert!(env.contains("SECRET=hunter2\n"));
    assert!(env.contains("PORT="));

    let bad = h
        .put_with_cookie(
            "/api/apps/ledger/env",
            r#"[{"key":"1BAD","value":"x"}]"#,
            &cookie,
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn updating_an_app_reprovisions_it() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.pretend_toolchain(RuntimeKind::Node, "22.11.0").await;
    h.post_with_cookie("/api/apps", &new_app_json("ledger"), &cookie)
        .await;
    let res = h
        .patch_with_cookie(
            "/api/apps/ledger",
            r#"{"memory_mb":1024,"routes":[{"path":"/","port_name":"main"},{"path":"/ws","port_name":"ws","websocket":true}]}"#,
            &cookie,
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["memory_mb"], 1024);
    let unit = h
        .platform
        .written("/etc/systemd/system/ferrum-app-ledger.service")
        .unwrap();
    assert!(unit.contains("MemoryMax=1024M"));
    let vhost = h
        .platform
        .written("/etc/nginx/conf.d/ferrum-ledger.conf")
        .unwrap();
    assert!(vhost.contains("location /ws {"));

    let bad = h
        .patch_with_cookie("/api/apps/ledger", r#"{"memory_mb":1}"#, &cookie)
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn deleting_requires_the_name_typed_back() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.pretend_toolchain(RuntimeKind::Node, "22.11.0").await;
    h.post_with_cookie("/api/apps", &new_app_json("ledger"), &cookie)
        .await;

    assert_eq!(
        h.delete_json_with_cookie("/api/apps/ledger", r#"{"name":"wrong"}"#, &cookie)
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.delete_json_with_cookie("/api/apps/ledger", r#"{"name":"ledger"}"#, &cookie)
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert!(
        h.platform
            .calls()
            .contains(&"remove_system_user ferrum-ledger".to_string())
    );
    assert_eq!(
        h.get_with_cookie("/api/apps/ledger", &cookie).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_bad_package_name_is_refused_before_anything_touches_the_host() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.pretend_toolchain(RuntimeKind::Node, "22.11.0").await;
    let mut body: serde_json::Value = serde_json::from_str(&new_app_json("ledger")).unwrap();
    body["packages"] = serde_json::json!(["ffmpeg", "libvips; rm -rf /"]);
    let res = h
        .post_with_cookie("/api/apps", &body.to_string(), &cookie)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.json);
    assert!(
        res.json["error"]
            .as_str()
            .unwrap()
            .contains("libvips; rm -rf /")
    );
    assert!(h.platform.calls().is_empty());
}

#[tokio::test]
async fn packages_are_installed_before_the_app_exists() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.pretend_toolchain(RuntimeKind::Node, "22.11.0").await;
    let mut body: serde_json::Value = serde_json::from_str(&new_app_json("ledger")).unwrap();
    body["packages"] = serde_json::json!(["ffmpeg"]);
    let res = h
        .post_with_cookie("/api/apps", &body.to_string(), &cookie)
        .await;
    assert_eq!(res.status, StatusCode::CREATED, "{}", res.json);
    assert_eq!(res.json["packages"], serde_json::json!(["ffmpeg"]));
    let calls = h.platform.calls();
    let install = calls
        .iter()
        .position(|c| c == "install_packages ffmpeg")
        .unwrap();
    let user = calls
        .iter()
        .position(|c| c.starts_with("create_system_user"))
        .unwrap();
    assert!(install < user, "{calls:#?}");
}

#[tokio::test]
async fn a_host_that_refuses_the_vhost_leaves_no_app_behind() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.pretend_toolchain(RuntimeKind::Node, "22.11.0").await;
    h.platform.fail_next("nginx_test");
    let res = h
        .post_with_cookie("/api/apps", &new_app_json("ledger"), &cookie)
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "{}", res.json);
    assert!(res.json["error"].as_str().unwrap().contains("nginx"));
    assert_eq!(
        h.get_with_cookie("/api/apps/ledger", &cookie).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_duplicate_slug_is_a_conflict() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.pretend_toolchain(RuntimeKind::Node, "22.11.0").await;
    h.post_with_cookie("/api/apps", &new_app_json("ledger"), &cookie)
        .await;
    let mut body: serde_json::Value = serde_json::from_str(&new_app_json("ledger")).unwrap();
    body["domains"] = serde_json::json!(["other.example.com"]);
    let res = h
        .post_with_cookie("/api/apps", &body.to_string(), &cookie)
        .await;
    assert_eq!(res.status, StatusCode::CONFLICT, "{}", res.json);
}

#[tokio::test]
async fn a_read_only_token_can_list_apps_and_nothing_else() {
    let (h, _cookie, _github) = signed_in_and_connected().await;
    let token = h.machine_token(true).await;
    assert_eq!(
        h.get_with_bearer("/api/apps", &token).await.status,
        StatusCode::OK
    );
    assert_eq!(
        h.post_with_bearer("/api/apps", &new_app_json("x"), &token)
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn apps_need_a_session() {
    let (h, _cookie, _github) = signed_in_and_connected().await;
    assert_eq!(h.get("/api/apps").await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        h.post("/api/apps/detect", "{}").await.status,
        StatusCode::UNAUTHORIZED
    );
}
