use super::{HOST, INTERVAL, Sample, cpu, now, prune, record};
use crate::apps::{self, App};
use crate::deploy;
use crate::events::{self, Kind};
use crate::state::State;
use ferrum_platform::{Platform, ProcStat};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

const PRUNE_EVERY_TICKS: u64 = 360;
/// A unit in a restart loop is up for a second at a time; it has not recovered until it stays up.
const RECOVERED_AFTER: Duration = Duration::from_secs(60);

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
    broken: HashSet<String>,
    up_since: HashMap<String, Instant>,
    recovered_after: Duration,
    ticks: u64,
}

impl Sampler {
    pub fn new(state: State, platform: Arc<dyn Platform>) -> Self {
        Self {
            state,
            platform,
            host: None,
            units: HashMap::new(),
            broken: HashSet::new(),
            up_since: HashMap::new(),
            recovered_after: RECOVERED_AFTER,
            ticks: 0,
        }
    }

    fn saw_up(&mut self, unit: &str, at: Instant) {
        let since = *self.up_since.entry(unit.to_string()).or_insert(at);
        if at.duration_since(since) >= self.recovered_after {
            self.broken.remove(unit);
        }
    }

    fn saw_down(&mut self, unit: &str) {
        self.up_since.remove(unit);
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
        let data_dir = self.state.data_dir.clone();
        let disk = tokio::task::spawn_blocking(move || platform.disk_usage(&data_dir))
            .await?
            .map_err(|e| tracing::warn!(error = %e, "reading disk usage"))
            .ok();
        let sample = Sample {
            at: now(),
            cpu_pct: cpu::percent(&prev.stat, &stat),
            memory_bytes: mem.total_kb.saturating_sub(mem.available_kb) * 1024,
            memory_peak_bytes: None,
            disk_used_bytes: disk.map(|d| d.used_bytes),
            net_rx_bytes: Some(net.0.saturating_sub(prev.net.0)),
            net_tx_bytes: Some(net.1.saturating_sub(prev.net.1)),
        };
        record(&self.state, HOST, &sample).await
    }

    /// One sample per app: its processes' memory added up, their CPU shares added up.
    async fn sample_apps(&mut self) -> anyhow::Result<()> {
        let at = now();
        for app in apps::list(&self.state).await? {
            let mut memory = 0;
            let mut peak = 0;
            let mut cpu_pct = 0.0;
            let mut ready = false;
            for process in app.command_processes() {
                let unit = process.unit_name(&app.slug);
                let Some(stats) = self.platform.cgroup_stats(&unit)? else {
                    self.units.remove(&unit);
                    self.saw_down(&unit);
                    self.watch(&app, &unit).await?;
                    continue;
                };
                let seen = Instant::now();
                self.saw_up(&unit, seen);
                memory += stats.memory_current;
                peak += stats.memory_peak;
                if let Some((prev_usec, prev_at)) =
                    self.units.insert(unit, (stats.cpu_usage_usec, seen))
                {
                    cpu_pct += cpu::cgroup_percent(prev_usec, stats.cpu_usage_usec, seen - prev_at);
                    ready = true;
                }
            }
            if !ready {
                continue;
            }
            let sample = Sample {
                at,
                cpu_pct,
                memory_bytes: memory,
                memory_peak_bytes: Some(peak),
                disk_used_bytes: None,
                net_rx_bytes: None,
                net_tx_bytes: None,
            };
            record(&self.state, &app.id, &sample).await?;
        }
        Ok(())
    }

    /// A released app whose unit is down while no deploy holds it broke on its own; said once.
    async fn watch(&mut self, app: &App, unit: &str) -> anyhow::Result<()> {
        if app.current_release_id.is_none() {
            self.broken.remove(unit);
            return Ok(());
        }
        let platform = self.platform.clone();
        let name = unit.to_string();
        if tokio::task::spawn_blocking(move || platform.service_is_active(&name)).await? {
            self.saw_up(unit, Instant::now());
            return Ok(());
        }
        if self.broken.contains(unit) || deploy::running_for(&self.state, &app.id).await?.is_some()
        {
            return Ok(());
        }
        self.broken.insert(unit.to_string());
        let link = format!("/apps/{}", app.slug);
        events::emit(
            &self.state,
            Kind::BrokeOnItsOwn,
            Some(&app.id),
            &app.slug,
            &format!(
                "{} is not running ({}), and no deploy stopped it.",
                app.name,
                unit.rsplit('-').next().unwrap_or(unit)
            ),
            Some(&link),
        )
        .await;
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
        docs.processes = vec![crate::apps::processes::NewProcess::folder("web", "dist")];
        apps::create(&state, docs).await.unwrap();
        platform.set_cgroup(
            "ferrum-app-ledger-web",
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
            "ferrum-app-ledger-web",
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
        let expected = format!("disk_usage {}", state.data_dir.display());
        assert!(platform.calls().iter().any(|c| c == &expected));
        platform.fail_next("disk_usage");
        sampler.tick().await.unwrap();
        assert_eq!(
            latest(&state, HOST).await.unwrap().unwrap().disk_used_bytes,
            None,
            "a failed df loses the disk column, not the sample"
        );
        assert!(series(&state, HOST, 3600, 60).await.unwrap().t.len() == 1);

        platform.clear_cgroup("ferrum-app-ledger-web");
        sampler.tick().await.unwrap();
        assert_eq!(
            latest(&state, &app.id).await.unwrap().unwrap().at,
            ledger.at
        );
        assert!(sampler.units.is_empty(), "a stopped unit forgets its delta");
    }

    async fn broke(state: &State) -> usize {
        events::list(state, 50, false)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == "broke_on_its_own")
            .count()
    }

    #[tokio::test]
    async fn a_released_app_that_stops_outside_a_deploy_is_reported_once_until_it_runs_again() {
        let (dir, state) = state().await;
        let platform = Arc::new(FakePlatform::new());
        let unit = "ferrum-app-ledger-web";
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let mut sampler = Sampler::new(state.clone(), platform.clone());

        sampler.tick().await.unwrap();
        assert_eq!(broke(&state).await, 0, "never released, never running");

        let release = deploy::releases::record(&state, &app, dir.path(), "main", "abc", None)
            .await
            .unwrap();
        deploy::releases::set_current(&state, &app.id, Some(&release.id))
            .await
            .unwrap();
        let running = deploy::create(
            &state,
            &app,
            deploy::Trigger::Manual,
            "main",
            &deploy::Commit::default(),
        )
        .await
        .unwrap();
        deploy::enter(&state, &running.id, deploy::DeployState::Restarting)
            .await
            .unwrap();
        sampler.tick().await.unwrap();
        assert_eq!(broke(&state).await, 0, "a deploy is restarting it");

        deploy::finish(&state, &running.id, deploy::Outcome::Live, None, None)
            .await
            .unwrap();
        sampler.tick().await.unwrap();
        sampler.tick().await.unwrap();
        assert_eq!(broke(&state).await, 1);
        let event = &events::list(&state, 1, false).await.unwrap()[0];
        assert_eq!(event.app_id.as_deref(), Some(app.id.as_str()));
        assert_eq!(event.link.as_deref(), Some("/apps/ledger"));

        platform.set_active(unit);
        sampler.tick().await.unwrap();
        platform.set_inactive(unit);
        sampler.tick().await.unwrap();
        assert_eq!(
            broke(&state).await,
            1,
            "a second of life is a restart loop, not a recovery"
        );

        sampler.recovered_after = Duration::ZERO;
        platform.set_active(unit);
        sampler.tick().await.unwrap();
        platform.set_inactive(unit);
        sampler.tick().await.unwrap();
        assert_eq!(broke(&state).await, 2, "staying up re-arms it");
    }
}
