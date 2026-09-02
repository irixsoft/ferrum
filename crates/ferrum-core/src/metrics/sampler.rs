use super::{HOST, INTERVAL, Sample, cpu, now, prune, record};
use crate::apps::{self, unit::unit_name};
use crate::state::State;
use ferrum_platform::{Platform, ProcStat};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::task::JoinHandle;

const PRUNE_EVERY_TICKS: u64 = 360;

struct Reading {
    stat: ProcStat,
    net: (u64, u64),
}

/// Keeps the previous readings so each tick records a delta; the first tick only remembers.
pub struct Sampler {
    state: State,
    platform: Arc<dyn Platform>,
    host: Option<Reading>,
    units: HashMap<String, (u64, Instant)>,
    ticks: u64,
}

impl Sampler {
    pub fn new(state: State, platform: Arc<dyn Platform>) -> Self {
        Self {
            state,
            platform,
            host: None,
            units: HashMap::new(),
            ticks: 0,
        }
    }

    pub async fn tick(&mut self) -> anyhow::Result<()> {
        self.ticks += 1;
        self.sample_host().await?;
        self.sample_apps().await?;
        if self.ticks.is_multiple_of(PRUNE_EVERY_TICKS) {
            prune(&self.state).await?;
        }
        Ok(())
    }

    async fn sample_host(&mut self) -> anyhow::Result<()> {
        let stat = self.platform.proc_stat()?;
        let net = self.platform.net_bytes()?;
        let next = Reading { stat, net };
        let Some(prev) = self.host.replace(next) else {
            return Ok(());
        };
        let mem = self.platform.proc_meminfo()?;
        let platform = self.platform.clone();
        let disk =
            tokio::task::spawn_blocking(move || platform.disk_usage(Path::new(crate::DATA_DIR)))
                .await??;
        let sample = Sample {
            at: now(),
            cpu_pct: cpu::percent(&prev.stat, &stat),
            memory_bytes: mem.total_kb.saturating_sub(mem.available_kb) * 1024,
            memory_peak_bytes: None,
            disk_used_bytes: Some(disk.used_bytes),
            net_rx_bytes: Some(net.0.saturating_sub(prev.net.0)),
            net_tx_bytes: Some(net.1.saturating_sub(prev.net.1)),
        };
        record(&self.state, HOST, &sample).await
    }

    async fn sample_apps(&mut self) -> anyhow::Result<()> {
        let at = now();
        for app in apps::list(&self.state).await? {
            if !app.runtime.has_process() {
                continue;
            }
            let unit = unit_name(&app.slug);
            let Some(stats) = self.platform.cgroup_stats(&unit)? else {
                self.units.remove(&unit);
                continue;
            };
            let seen = Instant::now();
            let Some((prev_usec, prev_at)) = self.units.insert(unit, (stats.cpu_usage_usec, seen))
            else {
                continue;
            };
            let sample = Sample {
                at,
                cpu_pct: cpu::cgroup_percent(prev_usec, stats.cpu_usage_usec, seen - prev_at),
                memory_bytes: stats.memory_current,
                memory_peak_bytes: Some(stats.memory_peak),
                disk_used_bytes: None,
                net_rx_bytes: None,
                net_tx_bytes: None,
            };
            record(&self.state, &app.id, &sample).await?;
        }
        Ok(())
    }
}

pub fn spawn_sampler(state: State, platform: Arc<dyn Platform>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut sampler = Sampler::new(state, platform);
        let mut interval = tokio::time::interval(INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(e) = sampler.tick().await {
                tracing::warn!(error = ?e, "sampling metrics");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::metrics::{latest, series};
    use ferrum_platform::{CgroupStats, FakePlatform};

    #[tokio::test]
    async fn each_tick_after_the_first_records_the_host_and_every_running_app() {
        let (_d, state) = state().await;
        let platform = Arc::new(FakePlatform::new());
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let mut docs = new_app("docs", &[("/", "main", false)]);
        docs.runtime = crate::runtime::RuntimeKind::Static;
        docs.output_dir = Some("dist".into());
        apps::create(&state, docs).await.unwrap();
        platform.set_cgroup(
            "ferrum-app-ledger",
            CgroupStats {
                memory_current: 90_000_000,
                memory_peak: 120_000_000,
                cpu_usage_usec: 1_000_000,
            },
        );
        platform.set_net(1000, 2000);

        let mut sampler = Sampler::new(state.clone(), platform.clone());
        sampler.tick().await.unwrap();
        assert!(latest(&state, HOST).await.unwrap().is_none());
        assert!(latest(&state, &app.id).await.unwrap().is_none());

        platform.set_proc_stat(ProcStat {
            busy_ticks: 1400,
            total_ticks: 5000,
        });
        platform.set_net(1500, 2000);
        platform.set_cgroup(
            "ferrum-app-ledger",
            CgroupStats {
                memory_current: 95_000_000,
                memory_peak: 125_000_000,
                cpu_usage_usec: 1_500_000,
            },
        );
        sampler.tick().await.unwrap();
        let host = latest(&state, HOST).await.unwrap().unwrap();
        assert_eq!(host.cpu_pct, 40.0);
        assert_eq!(host.memory_bytes, 1_048_576 * 1024);
        assert_eq!(host.disk_used_bytes, Some(20 * 1024 * 1024 * 1024));
        assert_eq!(host.net_rx_bytes, Some(500));
        assert_eq!(host.net_tx_bytes, Some(0));
        let ledger = latest(&state, &app.id).await.unwrap().unwrap();
        assert_eq!(ledger.memory_bytes, 95_000_000);
        assert_eq!(ledger.memory_peak_bytes, Some(125_000_000));
        assert!(ledger.cpu_pct > 0.0);
        assert!(
            platform
                .calls()
                .iter()
                .any(|c| c == "disk_usage /var/lib/ferrum")
        );
        assert!(series(&state, HOST, 3600, 60).await.unwrap().t.len() == 1);

        platform.clear_cgroup("ferrum-app-ledger");
        sampler.tick().await.unwrap();
        assert_eq!(
            latest(&state, &app.id).await.unwrap().unwrap().at,
            ledger.at
        );
        assert!(sampler.units.is_empty(), "a stopped unit forgets its delta");
    }
}
