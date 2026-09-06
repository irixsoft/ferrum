mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use ferrum::mcp::{READ_ONLY, TOKEN_REQUIRED};
use ferrum_platform::Exit;
use serde_json::{Value, json};
use support::{HOSTNAME, Harness, Res, StubHealth, signed_in, signed_in_and_connected};

const READ_TOOLS: [&str; 12] = [
    "app_logs",
    "build_log",
    "certificate_status",
    "database_info",
    "deploy_history",
    "deploy_log",
    "get_app",
    "list_apps",
    "list_databases",
    "metrics",
    "nginx_config",
    "system_status",
];

const WRITE_TOOLS: [&str; 8] = [
    "add_domain",
    "adjust_resource_limits",
    "create_database",
    "deploy",
    "edit_nginx_directives",
    "restart_app",
    "rollback",
    "set_env",
];

fn rpc(id: u32, method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string()
}

fn initialize() -> String {
    rpc(
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0" },
        }),
    )
}

async fn mcp_with_host(h: &Harness, auth: Option<(&str, &str)>, host: &str, body: &str) -> Res {
    let mut req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, host)
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some((name, value)) = auth {
        req = req.header(name, value);
    }
    h.send(req.body(Body::from(body.to_string())).unwrap())
        .await
}

async fn mcp(h: &Harness, token: &str, body: &str) -> Res {
    let bearer = format!("Bearer {token}");
    mcp_with_host(h, Some(("authorization", &bearer)), HOSTNAME, body).await
}

async fn tools(h: &Harness, token: &str) -> Vec<Value> {
    let res = mcp(h, token, &rpc(2, "tools/list", json!({}))).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    res.json["result"]["tools"].as_array().unwrap().clone()
}

async fn call(h: &Harness, token: &str, name: &str, args: Value) -> Value {
    let res = mcp(
        h,
        token,
        &rpc(3, "tools/call", json!({ "name": name, "arguments": args })),
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{name}: {}", res.text);
    assert!(
        res.json["error"].is_null(),
        "{name} answered a protocol error: {}",
        res.json
    );
    res.json["result"].clone()
}

fn error_text(result: &Value) -> &str {
    assert_eq!(result["isError"], true, "{result}");
    result["content"][0]["text"].as_str().unwrap()
}

fn names(tools: &[Value]) -> Vec<&str> {
    tools.iter().map(|t| t["name"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn the_endpoint_takes_a_bearer_token_and_the_panel_hostname_only() {
    let (h, cookie) = signed_in().await;
    let token = h.machine_token(false).await;

    let res = mcp(&h, &token, &initialize()).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert!(
        res.header("content-type")
            .unwrap()
            .starts_with("application/json"),
        "{:?}",
        res.headers
    );
    assert_eq!(res.json["result"]["serverInfo"]["name"], "ferrum");
    assert!(res.json["result"]["capabilities"]["tools"].is_object());
    assert!(
        res.json["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("panel")
    );

    let session = format!("ferrum_session={cookie}");
    let with_cookie = mcp_with_host(&h, Some(("cookie", &session)), HOSTNAME, &initialize()).await;
    assert_eq!(with_cookie.status, StatusCode::UNAUTHORIZED);
    assert_eq!(with_cookie.json["error"], TOKEN_REQUIRED);
    let anonymous = mcp_with_host(&h, None, HOSTNAME, &initialize()).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    let forged = mcp_with_host(
        &h,
        Some(("authorization", "Bearer ferr_nope")),
        HOSTNAME,
        &initialize(),
    )
    .await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);
    assert_eq!(forged.json["error"], "That API token is not valid.");

    let bearer = format!("Bearer {token}");
    let rebound = mcp_with_host(
        &h,
        Some(("authorization", &bearer)),
        "evil.example",
        &initialize(),
    )
    .await;
    assert_eq!(rebound.status, StatusCode::FORBIDDEN, "{}", rebound.text);
    let loopback = mcp_with_host(
        &h,
        Some(("authorization", &bearer)),
        "127.0.0.1:8443",
        &initialize(),
    )
    .await;
    assert_eq!(loopback.status, StatusCode::OK, "{}", loopback.text);
}

#[tokio::test]
async fn a_read_only_token_sees_the_read_tools_and_cannot_name_a_write_one() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    let read_only = h.machine_token(true).await;
    let read_write = h.machine_token(false).await;

    let visible = tools(&h, &read_only).await;
    assert_eq!(names(&visible), READ_TOOLS);
    for tool in &visible {
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{tool}");
        assert_eq!(tool["annotations"]["openWorldHint"], false, "{tool}");
        assert!(tool["description"].as_str().unwrap().len() > 20, "{tool}");
    }

    let all = tools(&h, &read_write).await;
    let mut expected: Vec<&str> = READ_TOOLS
        .iter()
        .chain(WRITE_TOOLS.iter())
        .copied()
        .collect();
    expected.sort_unstable();
    assert_eq!(names(&all), expected);
    for tool in all
        .iter()
        .filter(|t| WRITE_TOOLS.contains(&t["name"].as_str().unwrap()))
    {
        assert_eq!(tool["annotations"]["readOnlyHint"], false, "{tool}");
        assert_eq!(tool["annotations"]["destructiveHint"], false, "{tool}");
        assert!(tool["annotations"]["idempotentHint"].is_boolean(), "{tool}");
        assert!(
            tool["inputSchema"]["properties"]["slug"].is_object()
                || tool["name"] == "create_database",
            "{tool}"
        );
    }

    let before = h.platform.calls_matching("service").len();
    let refused = call(&h, &read_only, "restart_app", json!({ "slug": "ledger" })).await;
    assert_eq!(error_text(&refused), READ_ONLY);
    assert_eq!(h.platform.calls_matching("service").len(), before);
    let listed = call(&h, &read_only, "list_apps", json!({})).await;
    assert_eq!(listed["structuredContent"][0]["slug"], "ledger");
}

#[tokio::test]
async fn the_read_tools_answer_what_the_routes_answer_and_leak_no_secret() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.platform.set_postgres_major(18);
    h.create_app("ledger", &cookie).await;
    let env = h
        .put_with_cookie(
            "/api/apps/ledger/env",
            r#"[{"key":"SESSION_SECRET","value":"hunter2-plain"}]"#,
            &cookie,
        )
        .await;
    assert_eq!(env.status, StatusCode::NO_CONTENT, "{}", env.json);
    let db = h
        .post_with_cookie(
            "/api/databases",
            r#"{"name":"ledger_prod","app_slug":"ledger"}"#,
            &cookie,
        )
        .await;
    assert_eq!(db.status, StatusCode::CREATED, "{}", db.json);
    let health = StubHealth::start(200).await;
    h.force_port("ledger", health.port).await;
    h.platform
        .script_run("bun run build", &["Compiled"], Exit::Code(0));
    h.platform.set_active("ferrum-app-ledger");
    let live = h
        .post_with_cookie("/api/apps/ledger/deploys", "", &cookie)
        .await;
    let live_id = live.json["id"].as_str().unwrap().to_string();
    assert_eq!(
        h.wait_for_deploy(&live_id, &cookie).await["outcome"],
        "Live"
    );
    h.platform
        .script_run("bun run build", &["Type error in app.ts"], Exit::Code(1));
    let failed = h
        .post_with_cookie("/api/apps/ledger/deploys", "", &cookie)
        .await;
    let failed_id = failed.json["id"].as_str().unwrap().to_string();
    assert_eq!(
        h.wait_for_deploy(&failed_id, &cookie).await["outcome"],
        "Failed"
    );

    let token = h.machine_token(true).await;
    let mut transcript = String::new();

    for (name, args, route) in [
        ("list_apps", json!({}), "/api/apps"),
        ("get_app", json!({ "slug": "ledger" }), "/api/apps/ledger"),
        (
            "deploy_history",
            json!({ "slug": "ledger" }),
            "/api/apps/ledger/deploys",
        ),
        (
            "app_logs",
            json!({ "slug": "ledger", "source": "error", "lines": 50 }),
            "/api/apps/ledger/logs?source=error&lines=50",
        ),
        (
            "metrics",
            json!({ "scope": "host", "range": "1h" }),
            "/api/metrics?range=1h",
        ),
        (
            "metrics",
            json!({ "scope": "ledger" }),
            "/api/apps/ledger/metrics",
        ),
        (
            "nginx_config",
            json!({ "slug": "ledger" }),
            "/api/apps/ledger/nginx",
        ),
        ("list_databases", json!({}), "/api/databases"),
        (
            "database_info",
            json!({ "name": "ledger_prod" }),
            "/api/databases/ledger_prod",
        ),
    ] {
        let result = call(&h, &token, name, args.clone()).await;
        assert!(result["isError"] != true, "{name}: {result}");
        transcript.push_str(&result.to_string());
        let via_route = h.get_with_bearer(route, &token).await;
        assert_eq!(
            via_route.status,
            StatusCode::OK,
            "{route}: {}",
            via_route.json
        );
        assert_eq!(
            result["structuredContent"], via_route.json,
            "{name} {args} differs from {route}"
        );
    }

    let log = call(&h, &token, "deploy_log", json!({ "deploy_id": failed_id })).await;
    transcript.push_str(&log.to_string());
    let lines = log["structuredContent"].as_array().unwrap();
    assert!(lines.iter().any(|l| l["stream"] == "system"), "{log}");
    assert!(
        lines
            .iter()
            .any(|l| l["text"].as_str().unwrap().contains("Type error")),
        "{log}"
    );
    let build = call(&h, &token, "build_log", json!({ "deploy_id": live_id })).await;
    transcript.push_str(&build.to_string());
    let lines = build["structuredContent"].as_array().unwrap();
    assert!(!lines.is_empty());
    assert!(lines.iter().all(|l| l["stream"] != "system"), "{build}");
    assert!(lines.iter().any(|l| l["text"] == "Compiled"), "{build}");

    let certificates = call(
        &h,
        &token,
        "certificate_status",
        json!({ "slug": "ledger" }),
    )
    .await;
    transcript.push_str(&certificates.to_string());
    let app = h.get_with_bearer("/api/apps/ledger", &token).await;
    assert_eq!(certificates["structuredContent"], app.json["certificates"]);

    let system = call(&h, &token, "system_status", json!({})).await;
    transcript.push_str(&system.to_string());
    let host = h.get_with_bearer("/api/host", &token).await;
    let security = h.get_with_bearer("/api/security", &token).await;
    assert_eq!(
        system["structuredContent"]["hostname"],
        host.json["hostname"]
    );
    assert_eq!(
        system["structuredContent"]["ferrum_version"],
        host.json["ferrum_version"]
    );
    assert_eq!(system["structuredContent"]["security"], security.json);

    for forbidden in ["v1:", "PRIVATE KEY", "whsec", "cs_abc", "hunter2", "ghs_"] {
        assert!(
            !transcript.contains(forbidden),
            "{forbidden} reached a tool result"
        );
    }
    let urls: Vec<&str> = transcript
        .match_indices("postgres://")
        .map(|(i, _)| &transcript[i..i + 60])
        .collect();
    assert!(!urls.is_empty());
    for url in urls {
        assert!(url.contains("<password>@"), "{url}");
    }

    let missing = call(&h, &token, "get_app", json!({ "slug": "nope" })).await;
    assert_eq!(error_text(&missing), "No such application.");
    let bad_range = call(
        &h,
        &token,
        "metrics",
        json!({ "scope": "host", "range": "1y" }),
    )
    .await;
    assert_eq!(
        error_text(&bad_range),
        "range must be 1h, 24h or 7d, not 1y."
    );
    let bad_args = call(&h, &token, "get_app", json!({})).await;
    assert!(error_text(&bad_args).contains("slug"), "{bad_args}");
    let unknown = mcp(
        &h,
        &token,
        &rpc(
            9,
            "tools/call",
            json!({ "name": "delete_app", "arguments": {} }),
        ),
    )
    .await;
    assert_eq!(unknown.status, StatusCode::OK, "{}", unknown.text);
    assert!(!unknown.json["error"].is_null(), "{}", unknown.json);
}

const CUSTOM: &str = "/etc/nginx/ferrum-custom/ledger.conf";
const UNIT: &str = "/etc/systemd/system/ferrum-app-ledger.service";

#[tokio::test]
async fn the_write_tools_change_the_box_the_way_the_routes_do() {
    let (h, cookie, _github) = signed_in_and_connected().await;
    h.platform.set_postgres_major(18);
    h.create_app("ledger", &cookie).await;
    let token = h.machine_token(false).await;

    let set = call(
        &h,
        &token,
        "set_env",
        json!({ "slug": "ledger", "vars": [{ "key": "FOO", "value": "bar" }] }),
    )
    .await;
    assert_eq!(set["structuredContent"]["keys"], 1, "{set}");
    assert!(h.env_file("ledger").contains("FOO=bar"));
    let shown = call(&h, &token, "get_app", json!({ "slug": "ledger" })).await;
    assert_eq!(shown["structuredContent"]["env"][0]["key"], "FOO");
    assert!(!shown.to_string().contains("bar\""), "{shown}");
    let bad_key = call(
        &h,
        &token,
        "set_env",
        json!({ "slug": "ledger", "vars": [{ "key": "1BAD", "value": "x" }] }),
    )
    .await;
    assert!(error_text(&bad_key).contains("1BAD"), "{bad_key}");

    let created = call(
        &h,
        &token,
        "create_database",
        json!({ "name": "ledger_prod", "app_slug": "ledger" }),
    )
    .await;
    assert_eq!(created["structuredContent"]["name"], "ledger_prod");
    assert_eq!(created["structuredContent"]["linked_apps"][0], "ledger");
    assert!(h.env_file("ledger").contains("DATABASE_URL="));
    assert!(!created.to_string().contains("v1:"));
    let taken = call(
        &h,
        &token,
        "create_database",
        json!({ "name": "ledger_prod" }),
    )
    .await;
    assert!(error_text(&taken).contains("ledger_prod"), "{taken}");

    let health = StubHealth::start(200).await;
    h.force_port("ledger", health.port).await;
    h.platform
        .script_run("bun run build", &["Compiled"], Exit::Code(0));
    h.platform.set_active("ferrum-app-ledger");
    let early = call(&h, &token, "restart_app", json!({ "slug": "ledger" })).await;
    assert!(error_text(&early).contains("not been deployed"), "{early}");

    let queued = call(&h, &token, "deploy", json!({ "slug": "ledger" })).await;
    assert_eq!(queued["structuredContent"]["trigger"], "manual");
    assert_eq!(queued["structuredContent"]["git_ref"], "main");
    let first_id = queued["structuredContent"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        h.wait_for_deploy(&first_id, &cookie).await["outcome"],
        "Live"
    );
    let log = call(&h, &token, "deploy_log", json!({ "deploy_id": first_id })).await;
    assert!(log.to_string().contains("Compiled"), "{log}");
    let second = call(
        &h,
        &token,
        "deploy",
        json!({ "slug": "ledger", "git_ref": "main" }),
    )
    .await;
    let second_id = second["structuredContent"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let second = h.wait_for_deploy(&second_id, &cookie).await;
    assert_eq!(second["outcome"], "Live", "{second}");

    let restarts = h
        .platform
        .calls_matching("service restart ferrum-app-ledger")
        .len();
    let restarted = call(&h, &token, "restart_app", json!({ "slug": "ledger" })).await;
    assert_eq!(restarted["structuredContent"]["restarted"], true);
    assert_eq!(
        h.platform
            .calls_matching("service restart ferrum-app-ledger")
            .len(),
        restarts + 1
    );

    let history = call(
        &h,
        &token,
        "deploy_history",
        json!({ "slug": "ledger", "limit": 1 }),
    )
    .await;
    assert_eq!(history["structuredContent"].as_array().unwrap().len(), 1);
    let releases = h
        .get_with_cookie("/api/apps/ledger/releases", &cookie)
        .await;
    let first_release = releases.json[1]["id"].as_str().unwrap().to_string();
    let no_snapshot = call(
        &h,
        &token,
        "rollback",
        json!({ "slug": "ledger", "release_id": first_release, "restore_deploy_id": second_id }),
    )
    .await;
    assert_eq!(error_text(&no_snapshot), "That deploy took no snapshot.");
    let rolled = call(
        &h,
        &token,
        "rollback",
        json!({ "slug": "ledger", "release_id": first_release }),
    )
    .await;
    assert_eq!(rolled["structuredContent"]["trigger"], "rollback");
    let rollback_id = rolled["structuredContent"]["id"].as_str().unwrap();
    let done = h.wait_for_deploy(rollback_id, &cookie).await;
    assert_eq!(done["outcome"], "Live", "{done}");
    let app = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert_eq!(app.json["current_release"]["id"], first_release);

    let before = h.platform.written(CUSTOM).unwrap_or_default();
    h.platform.fail_next("nginx_test");
    let rejected = call(
        &h,
        &token,
        "edit_nginx_directives",
        json!({ "slug": "ledger", "custom": "location /x { return 418; }\n" }),
    )
    .await;
    assert_eq!(
        error_text(&rejected),
        "nginx rejected the file: scripted failure"
    );
    assert_eq!(h.platform.written(CUSTOM).unwrap_or_default(), before);
    let edited = call(
        &h,
        &token,
        "edit_nginx_directives",
        json!({ "slug": "ledger", "custom": "location /x { return 418; }\n" }),
    )
    .await;
    assert!(
        edited["structuredContent"]["custom"]
            .as_str()
            .unwrap()
            .starts_with("location /x"),
        "{edited}"
    );

    let added = call(
        &h,
        &token,
        "add_domain",
        json!({ "slug": "ledger", "domain": "Books.Example.com" }),
    )
    .await;
    assert_eq!(
        added["structuredContent"]["domains"],
        json!(["ledger.example.com", "books.example.com"])
    );
    let shown = call(&h, &token, "get_app", json!({ "slug": "ledger" })).await;
    assert_eq!(
        shown["structuredContent"]["domains"][1],
        "books.example.com"
    );
    let bad_domain = call(
        &h,
        &token,
        "add_domain",
        json!({ "slug": "ledger", "domain": "not a domain" }),
    )
    .await;
    assert_eq!(bad_domain["isError"], true, "{bad_domain}");

    let nothing = call(&h, &token, "adjust_resource_limits", json!({})).await;
    assert_eq!(error_text(&nothing), "Name at least one limit to change.");
    let mixed = call(
        &h,
        &token,
        "adjust_resource_limits",
        json!({ "slug": "ledger", "build_secs": 900 }),
    )
    .await;
    assert_eq!(mixed["isError"], true, "{mixed}");
    let app_limits = call(
        &h,
        &token,
        "adjust_resource_limits",
        json!({ "slug": "ledger", "memory_mb": 768 }),
    )
    .await;
    assert_eq!(app_limits["structuredContent"]["memory_mb"], 768);
    assert!(h.platform.written(UNIT).unwrap().contains("MemoryMax=768M"));
    let build_limits = call(
        &h,
        &token,
        "adjust_resource_limits",
        json!({ "build_memory_mb": 1024, "build_secs": 900 }),
    )
    .await;
    assert_eq!(build_limits["structuredContent"]["memory_mb"], 1024);
    assert_eq!(build_limits["structuredContent"]["build_secs"], 900);
    let settings = h.get_with_cookie("/api/settings/builds", &cookie).await;
    assert_eq!(settings.json["memory_mb"], 1024);
    assert_eq!(settings.json["build_secs"], 900);
    assert_eq!(settings.json["migrate_secs"], 600);
    let too_short = call(
        &h,
        &token,
        "adjust_resource_limits",
        json!({ "build_secs": 5 }),
    )
    .await;
    assert_eq!(too_short["isError"], true, "{too_short}");
}
