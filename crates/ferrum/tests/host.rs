mod support;

use axum::http::StatusCode;
use ferrum_platform::CgroupStats;
use support::{HOSTNAME, signed_in};

#[tokio::test]
async fn the_host_card_reads_the_box_and_says_what_is_not_there_yet() {
    let (h, cookie) = signed_in().await;
    h.platform.set_active("nginx");
    let res = h.get_with_cookie("/api/host", &cookie).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.json);
    assert_eq!(res.json["hostname"], HOSTNAME);
    assert_eq!(res.json["ferrum_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(res.json["arch"], std::env::consts::ARCH);
    assert_eq!(res.json["uptime_secs"], 3600);
    assert_eq!(res.json["cpu_cores"], 2);
    assert_eq!(res.json["cpu_pct"], 0.0);
    assert_eq!(res.json["memory_used_mb"], 1024);
    assert_eq!(res.json["memory_total_mb"], 2048);
    assert_eq!(res.json["disk_used_gb"], 20.0);
    assert_eq!(res.json["disk_total_gb"], 80.0);
    let services = res.json["services"].as_array().unwrap();
    assert_eq!(services.len(), 7);
    let named = |name: &str| {
        services
            .iter()
            .find(|s| s["name"] == name)
            .unwrap_or_else(|| panic!("no {name} service"))
    };
    assert_eq!(named("nginx")["detail"], "active");
    assert_eq!(named("PostgreSQL")["detail"], "not installed");
    assert_eq!(named("Redis")["detail"], "none");
    assert_eq!(named("Deploys")["detail"], "none yet");
    assert_eq!(named("fail2ban")["detail"], "not enabled");
    assert_eq!(named("fail2ban")["ok"], false);
    assert_eq!(named("ufw")["ok"], false);
    assert!(
        services
            .iter()
            .filter(|s| s["name"] != "fail2ban" && s["name"] != "ufw")
            .all(|s| s["ok"] == true),
        "{services:?}"
    );

    let token = h.machine_token(true).await;
    let read_only = h.get_with_bearer("/api/host", &token).await;
    assert_eq!(read_only.status, StatusCode::OK);
    assert_eq!(h.get("/api/host").await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_app_reports_its_cgroup_only_while_the_unit_exists() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    let idle = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert!(idle.json["memory_bytes"].is_null());
    assert!(idle.json["memory_peak_bytes"].is_null());
    assert!(idle.json["cpu_pct"].is_null());

    h.platform.set_cgroup(
        "ferrum-app-ledger",
        CgroupStats {
            memory_current: 90_000_000,
            memory_peak: 120_000_000,
            cpu_usage_usec: 5_000_000,
        },
    );
    let running = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert_eq!(running.json["memory_bytes"], 90_000_000);
    assert_eq!(running.json["memory_peak_bytes"], 120_000_000);
    assert_eq!(running.json["cpu_pct"], 0.0);
    let card = h.get_with_cookie("/api/host", &cookie).await;
    let certs = card.json["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "Certificates")
        .unwrap()
        .clone();
    assert_eq!(certs["ok"], false);
    assert!(
        certs["detail"]
            .as_str()
            .unwrap()
            .starts_with("ledger.example.com"),
        "{certs}"
    );
}
