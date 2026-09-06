mod support;

use axum::http::StatusCode;
use ferrum_core::metrics::Sampler;
use ferrum_platform::{CgroupStats, ProcStat};
use support::signed_in;

#[tokio::test]
async fn two_ticks_of_the_sampler_put_the_host_and_the_app_on_their_charts() {
    let (h, cookie) = signed_in().await;
    h.create_app("ledger", &cookie).await;
    h.platform.set_cgroup(
        "ferrum-app-ledger",
        CgroupStats {
            memory_current: 100 * 1024 * 1024,
            memory_peak: 150 * 1024 * 1024,
            cpu_usage_usec: 0,
        },
    );
    let empty = h.get_with_cookie("/api/metrics?range=1h", &cookie).await;
    assert_eq!(empty.status, StatusCode::OK, "{}", empty.json);
    assert!(empty.json["t"].as_array().unwrap().is_empty());
    assert!(empty.json["values"]["cpu"].as_array().unwrap().is_empty());

    let mut sampler = Sampler::new(h.db.clone(), h.platform.clone());
    sampler.tick().await.unwrap();
    h.platform.set_proc_stat(ProcStat {
        busy_ticks: 1300,
        total_ticks: 5000,
    });
    h.platform.set_cgroup(
        "ferrum-app-ledger",
        CgroupStats {
            memory_current: 100 * 1024 * 1024,
            memory_peak: 150 * 1024 * 1024,
            cpu_usage_usec: 250_000,
        },
    );
    sampler.tick().await.unwrap();

    let host = h.get_with_cookie("/api/metrics?range=1h", &cookie).await;
    assert_eq!(host.json["t"].as_array().unwrap().len(), 1);
    assert_eq!(host.json["values"]["cpu"][0], 30.0);
    assert_eq!(host.json["values"]["memory"][0], 50.0);

    let app = h
        .get_with_cookie("/api/apps/ledger/metrics?range=24h", &cookie)
        .await;
    assert_eq!(app.status, StatusCode::OK, "{}", app.json);
    assert_eq!(app.json["t"].as_array().unwrap().len(), 1);
    assert_eq!(app.json["values"]["memory"][0], 100.0);
    assert!(app.json["values"]["cpu"][0].as_f64().unwrap() > 0.0);

    let shown = h.get_with_cookie("/api/apps/ledger", &cookie).await;
    assert!(shown.json["cpu_pct"].as_f64().unwrap() > 0.0);
    let card = h.get_with_cookie("/api/host", &cookie).await;
    assert_eq!(card.json["cpu_pct"], 30.0);

    let bad = h.get_with_cookie("/api/metrics?range=1y", &cookie).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let missing = h.get_with_cookie("/api/apps/nope/metrics", &cookie).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    let token = h.machine_token(true).await;
    assert_eq!(
        h.get_with_bearer("/api/metrics?range=7d", &token)
            .await
            .status,
        StatusCode::OK
    );
}
