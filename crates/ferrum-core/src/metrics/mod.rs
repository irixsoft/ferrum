pub mod cpu;
pub mod sampler;

pub use sampler::{Sampler, spawn_sampler};

use crate::state::State;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::Duration;

pub const INTERVAL: Duration = Duration::from_secs(10);
pub const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub const HOST: &str = "host";
pub const POINTS: usize = 360;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sample {
    pub at: i64,
    pub cpu_pct: f64,
    pub memory_bytes: u64,
    pub memory_peak_bytes: Option<u64>,
    pub disk_used_bytes: Option<u64>,
    pub net_rx_bytes: Option<u64>,
    pub net_tx_bytes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Series {
    pub t: Vec<i64>,
    pub values: BTreeMap<&'static str, Vec<f64>>,
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub async fn record(state: &State, scope: &str, sample: &Sample) -> anyhow::Result<()> {
    let memory = sample.memory_bytes as i64;
    let peak = sample.memory_peak_bytes.map(|b| b as i64);
    let disk = sample.disk_used_bytes.map(|b| b as i64);
    let rx = sample.net_rx_bytes.map(|b| b as i64);
    let tx = sample.net_tx_bytes.map(|b| b as i64);
    sqlx::query!(
        "INSERT INTO metrics (at, scope, cpu_pct, memory_bytes, memory_peak_bytes, disk_used_bytes, net_rx_bytes, net_tx_bytes)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        sample.at,
        scope,
        sample.cpu_pct,
        memory,
        peak,
        disk,
        rx,
        tx
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Bucketed averages over the last `since_secs`, at most `points` of them; `cpu` in percent and
/// `memory_bytes` as they were sampled.
pub async fn series(
    state: &State,
    scope: &str,
    since_secs: u64,
    points: usize,
) -> anyhow::Result<Series> {
    let bucket = (since_secs / points.max(1) as u64).max(INTERVAL.as_secs()) as i64;
    let from = now() - since_secs as i64;
    let rows = sqlx::query!(
        r#"SELECT (at / ?) * ? AS "t!: i64", avg(cpu_pct) AS "cpu!: f64", avg(memory_bytes) AS "memory!: f64"
           FROM metrics WHERE scope = ? AND at >= ?
           GROUP BY at / ? ORDER BY 1"#,
        bucket,
        bucket,
        scope,
        from,
        bucket
    )
    .fetch_all(&state.pool)
    .await?;
    let mut out = Series::default();
    let (mut cpu, mut memory) = (Vec::new(), Vec::new());
    for row in rows {
        out.t.push(row.t);
        cpu.push((row.cpu * 10.0).round() / 10.0);
        memory.push(row.memory.round());
    }
    out.values.insert("cpu", cpu);
    out.values.insert("memory_bytes", memory);
    Ok(out)
}

pub async fn latest(state: &State, scope: &str) -> anyhow::Result<Option<Sample>> {
    let row = sqlx::query!(
        r#"SELECT at AS "at!: i64", cpu_pct AS "cpu_pct!: f64", memory_bytes AS "memory_bytes!: i64",
                  memory_peak_bytes, disk_used_bytes, net_rx_bytes, net_tx_bytes
           FROM metrics WHERE scope = ? ORDER BY at DESC LIMIT 1"#,
        scope
    )
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|r| Sample {
        at: r.at,
        cpu_pct: r.cpu_pct,
        memory_bytes: r.memory_bytes as u64,
        memory_peak_bytes: r.memory_peak_bytes.map(|b| b as u64),
        disk_used_bytes: r.disk_used_bytes.map(|b| b as u64),
        net_rx_bytes: r.net_rx_bytes.map(|b| b as u64),
        net_tx_bytes: r.net_tx_bytes.map(|b| b as u64),
    }))
}

pub async fn prune(state: &State) -> anyhow::Result<u64> {
    let cutoff = now() - RETENTION.as_secs() as i64;
    let done = sqlx::query!("DELETE FROM metrics WHERE at < ?", cutoff)
        .execute(&state.pool)
        .await?;
    Ok(done.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::state;

    pub fn sample(at: i64, cpu_pct: f64, memory_bytes: u64) -> Sample {
        Sample {
            at,
            cpu_pct,
            memory_bytes,
            memory_peak_bytes: None,
            disk_used_bytes: None,
            net_rx_bytes: None,
            net_tx_bytes: None,
        }
    }

    #[tokio::test]
    async fn a_series_buckets_samples_into_averages_and_never_more_points_than_asked() {
        let (_d, state) = state().await;
        let base = now() - 600;
        for i in 0..60 {
            record(&state, HOST, &sample(base + i * 10, 20.0 + i as f64, 1000))
                .await
                .unwrap();
        }
        record(&state, "other", &sample(base, 99.0, 5))
            .await
            .unwrap();
        let s = series(&state, HOST, 3600, 60).await.unwrap();
        assert_eq!(s.t.len(), s.values["cpu"].len());
        assert!(s.t.len() <= 60);
        assert!(s.t.len() >= 5, "{:?}", s.t);
        assert!(s.t.windows(2).all(|w| w[0] < w[1]));
        assert!(s.t.iter().all(|t| t % 60 == 0));
        assert_eq!(s.values["memory_bytes"][0], 1000.0);
        assert!(s.values["cpu"][0] >= 20.0 && s.values["cpu"][0] < 26.0);
        assert!(s.values["cpu"].iter().all(|c| *c != 99.0));
        let fine = series(&state, HOST, 600, 360).await.unwrap();
        assert_eq!(fine.t.len(), 60);
        assert_eq!(fine.values["cpu"][59], 79.0);
        assert!(series(&state, "nope", 3600, 60).await.unwrap().t.is_empty());
    }

    #[tokio::test]
    async fn latest_is_the_newest_row_and_prune_removes_only_old_ones() {
        let (_d, state) = state().await;
        let stale = now() - RETENTION.as_secs() as i64 - 60;
        record(&state, HOST, &sample(stale, 1.0, 1)).await.unwrap();
        record(&state, HOST, &sample(now() - 20, 2.0, 2))
            .await
            .unwrap();
        record(&state, HOST, &sample(now() - 10, 3.0, 3))
            .await
            .unwrap();
        assert_eq!(latest(&state, HOST).await.unwrap().unwrap().memory_bytes, 3);
        assert!(latest(&state, "nope").await.unwrap().is_none());
        assert_eq!(prune(&state).await.unwrap(), 1);
        assert_eq!(prune(&state).await.unwrap(), 0);
        let s = series(&state, HOST, 3600, 360).await.unwrap();
        let total: f64 = s.values["memory_bytes"].iter().sum::<f64>();
        assert!(
            (total - 5.0).abs() < 0.01 || (total - 2.5).abs() < 0.01,
            "only the two fresh rows remain: {s:?}"
        );
    }
}
