use super::log::Log;
use super::steps::Job;
use super::{Outcome, by_id};
use crate::apps;
use crate::github::Api;
use crate::runtime::toolchain::Store;
use crate::state::State;
use anyhow::Context;
use ferrum_platform::Platform;
use std::sync::Arc;
use std::time::Duration;

pub const BUILD_TIMEOUT: Duration = Duration::from_secs(20 * 60);
pub const MIGRATE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
pub const DISK_MIN_BYTES: u64 = 1024 * 1024 * 1024;
pub const BUILD_MEMORY_RESERVE_MB: u64 = 512;
pub const BUILD_MEMORY_FLOOR_MB: u64 = 512;
pub const CPU_WEIGHT: u32 = 50;
pub const IO_WEIGHT: u32 = 50;

#[derive(Clone)]
pub struct Ctx {
    pub state: State,
    pub platform: Arc<dyn Platform>,
    pub github: Api,
    pub http: reqwest::Client,
    pub log: Log,
    pub toolchains: Store,
    pub build_memory_mb: u64,
    pub build_timeout: Duration,
    pub migrate_timeout: Duration,
    pub health_interval: Duration,
}

impl Ctx {
    pub fn new(
        state: State,
        platform: Arc<dyn Platform>,
        github: Api,
        http: reqwest::Client,
        toolchains: Store,
    ) -> Self {
        let total_kb = platform.total_memory_kb().unwrap_or(0);
        Self {
            state,
            platform,
            github,
            http,
            log: Log::default(),
            toolchains,
            build_memory_mb: build_memory_mb(total_kb),
            build_timeout: BUILD_TIMEOUT,
            migrate_timeout: MIGRATE_TIMEOUT,
            health_interval: Duration::from_secs(1),
        }
    }
}

/// Everything but half a gigabyte, so the running apps and PostgreSQL keep theirs.
pub fn build_memory_mb(total_kb: u64) -> u64 {
    (total_kb / 1024)
        .saturating_sub(BUILD_MEMORY_RESERVE_MB)
        .max(BUILD_MEMORY_FLOOR_MB)
}

pub async fn run(ctx: &Ctx, deploy_id: &str) -> anyhow::Result<Outcome> {
    let deploy = by_id(&ctx.state, deploy_id)
        .await?
        .context("no such deploy")?;
    let app = apps::by_id(&ctx.state, &deploy.app_id)
        .await?
        .context("the application was deleted")?;
    let mut job = Job::new(ctx.clone(), app, deploy);
    let outcome = match job.pipeline().await {
        Ok(outcome) => outcome,
        Err(e) => job.abort(e).await?,
    };
    ctx.log.done(deploy_id, outcome);
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::apps::{self, App, NewApp, provision};
    use crate::deploy::tests::commit;
    use crate::deploy::{Commit, DeployState, StepStatus, Trigger, create, log, releases};
    use crate::postgres;
    use crate::runtime::RuntimeKind;
    use ferrum_platform::{Exit, FakePlatform};
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn the_build_limit_leaves_half_a_gigabyte_and_never_drops_below_it() {
        assert_eq!(build_memory_mb(2 * 1024 * 1024), 1536);
        assert_eq!(build_memory_mb(512 * 1024), 512);
        assert_eq!(build_memory_mb(0), 512);
    }

    /// A real listener inside the allocator's range, answering one status on every path.
    struct Health {
        port: u16,
        hits: Arc<AtomicUsize>,
    }

    impl Health {
        async fn serve(status: u16) -> Self {
            let listener = loop {
                let port = 20000 + (rand::random::<u16>() % 9999);
                if let Ok(l) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                    break l;
                }
            };
            let port = listener.local_addr().unwrap().port();
            let hits = Arc::new(AtomicUsize::new(0));
            let counter = hits.clone();
            tokio::spawn(async move {
                loop {
                    let Ok((mut socket, _)) = listener.accept().await else {
                        break;
                    };
                    counter.fetch_add(1, Ordering::SeqCst);
                    let mut buf = [0u8; 1024];
                    let _ = socket.read(&mut buf).await;
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                }
            });
            Self { port, hits }
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    fn ctx(state: &State, platform: &Arc<FakePlatform>) -> Ctx {
        let mut ctx = Ctx::new(
            state.clone(),
            platform.clone(),
            Api::at("http://127.0.0.1:1").with_fixed_token("ghs_fixed"),
            crate::http::client(),
            Store::at("/var/lib/ferrum/runtimes"),
        );
        ctx.health_interval = Duration::from_millis(20);
        ctx.build_memory_mb = 1200;
        ctx
    }

    async fn provisioned(
        state: &State,
        platform: &FakePlatform,
        slug: &str,
        port: u16,
        tweak: impl FnOnce(&mut NewApp),
    ) -> App {
        let mut new = new_app(slug, &[("/", "main", false)]);
        new.health.startup_budget_secs = 5;
        tweak(&mut new);
        let app = apps::create(state, new).await.unwrap();
        sqlx::query("UPDATE app_ports SET port = ? WHERE app_id = ? AND name = 'main'")
            .bind(port as i64)
            .bind(&app.id)
            .execute(&state.pool)
            .await
            .unwrap();
        let app = apps::by_slug(state, slug).await.unwrap().unwrap();
        provision::provision(state, platform, &app).await.unwrap();
        app
    }

    fn position(calls: &[String], needle: impl Fn(&str) -> bool) -> usize {
        calls
            .iter()
            .position(|c| needle(c))
            .unwrap_or_else(|| panic!("no matching call in {calls:#?}"))
    }

    async fn deploy(ctx: &Ctx, app: &App, sha: &str) -> (Outcome, crate::deploy::Deploy) {
        let d = create(&ctx.state, app, Trigger::Manual, "main", &commit(sha))
            .await
            .unwrap();
        let outcome = run(ctx, &d.id).await.unwrap();
        (outcome, by_id(&ctx.state, &d.id).await.unwrap().unwrap())
    }

    #[tokio::test]
    async fn a_full_deploy_runs_every_state_in_order_and_ends_live() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        p.set_head("a3f9c2d4e81b06f5c9a2");
        p.script_run("bun install", &["+ 412 packages"], Exit::Code(0));
        p.script_run("bun run build", &["Compiled"], Exit::Code(0));
        let health = Health::serve(200).await;
        let app = provisioned(&state, &p, "ledger", health.port, |_| {}).await;
        let ctx = ctx(&state, &p);
        let (outcome, d) = deploy(&ctx, &app, "a3f9c2d4e81b06f5c9a2").await;
        assert_eq!(outcome, Outcome::Live, "{:?}", d.failure_reason);

        let calls = p.calls();
        let clone = position(&calls, |c| {
            c.starts_with("git_clone https://github.com/irixsoft/ledger.git main /var/lib/ferrum/apps/ledger/releases/")
        });
        let checkout = position(&calls, |c| {
            c.starts_with("git_checkout") && c.ends_with("a3f9c2d4e81b06f5c9a2")
        });
        let scrub = position(&calls, |c| {
            c.starts_with("git_scrub_remote")
                && c.ends_with("https://github.com/irixsoft/ledger.git")
        });
        let env = calls
            .iter()
            .rposition(|c| c == "write_file /var/lib/ferrum/apps/ledger/shared/.env 600")
            .unwrap();
        let install = position(&calls, |c| {
            c.starts_with("run_scoped ferrum-build-ledger") && c.contains("bun install")
        });
        let build = position(&calls, |c| {
            c.starts_with("run_scoped") && c.contains("bun run build")
        });
        let swap = position(&calls, |c| {
            c.starts_with("symlink_swap /var/lib/ferrum/apps/ledger/releases/")
                && c.ends_with(" /var/lib/ferrum/apps/ledger/current")
        });
        let start = position(&calls, |c| c == "service enable-now ferrum-app-ledger");
        assert!(
            clone < checkout
                && checkout < scrub
                && scrub < env
                && env < install
                && install < build
                && build < swap
                && swap < start,
            "{calls:#?}"
        );
        assert!(
            !calls
                .iter()
                .any(|c| c.contains("postgres_dump") || c.contains("/apps/ledger/maintenance")),
            "no migration command, so no snapshot and no pause: {calls:#?}"
        );
        assert!(!calls.join("\n").contains("x-access-token"));
        assert!(health.hits() >= 1);

        assert_eq!(d.commit_sha.as_deref(), Some("a3f9c2d4e81b06f5c9a2"));
        let skipped: Vec<_> = d
            .steps
            .iter()
            .filter(|s| s.status == StepStatus::Skipped)
            .map(|s| s.state)
            .collect();
        assert_eq!(
            skipped,
            vec![
                DeployState::InstallingSystemPackages,
                DeployState::Snapshotting,
                DeployState::MaintenanceOn,
                DeployState::Migrating,
                DeployState::MaintenanceOff
            ]
        );
        assert!(
            d.steps
                .iter()
                .all(|s| s.status != StepStatus::Pending && s.status != StepStatus::Active)
        );
        let app = apps::by_slug(&state, "ledger").await.unwrap().unwrap();
        assert!(app.current_release_id.is_some());
        assert_eq!(app.current_release_id, d.release_id);
        let release = releases::by_id(&state, d.release_id.as_ref().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(release.current);
        assert!(release.dir.ends_with("_a3f9c2d"));
        assert_eq!(
            p.link("/var/lib/ferrum/apps/ledger/current").as_deref(),
            Some(release.dir.as_str())
        );

        let lines = log::lines(&state, &d.id, 0).await.unwrap();
        let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert!(text.contains(&"→ Building"), "{text:#?}");
        assert!(text.contains(&"Compiled"));
        assert!(text.contains(&"+ 412 packages"));
        assert!(text.iter().any(|l| l.starts_with("Live at a3f9c2d")));
    }

    #[tokio::test]
    async fn the_build_runs_as_the_app_user_with_the_env_file_the_toolchain_and_the_cache() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let health = Health::serve(200).await;
        let app = provisioned(&state, &p, "ledger", health.port, |_| {}).await;
        apps::env::set(&state, &app.id, "SECRET", "hunter2")
            .await
            .unwrap();
        postgres::create(&state, p.as_ref(), postgres::tests::new("ledger_prod"))
            .await
            .unwrap();
        postgres::link(&state, &app.id, "ledger_prod")
            .await
            .unwrap();
        sqlx::query("INSERT INTO toolchains (kind, version, path, size_bytes) VALUES ('bun', '1.2.3', '/x', 1)")
            .execute(&state.pool)
            .await
            .unwrap();
        let ctx = ctx(&state, &p);
        let (outcome, _) = deploy(&ctx, &app, "abc1234").await;
        assert_eq!(outcome, Outcome::Live);

        let runs = p.runs();
        assert_eq!(runs.len(), 2);
        let build = &runs[1];
        assert_eq!(build.user, "ferrum-ledger");
        assert!(
            build
                .cwd
                .starts_with("/var/lib/ferrum/apps/ledger/releases/")
        );
        assert_eq!(build.memory_max_mb, 1200);
        assert_eq!(build.cpu_weight, 50);
        assert_eq!(build.unit.len(), "ferrum-build-ledger-".len() + 7);
        let get = |k: &str| {
            build
                .env
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(
            get("PATH").unwrap(),
            "/var/lib/ferrum/runtimes/bun/1.2.3:/var/lib/ferrum/runtimes/node/22.11.0/bin:/usr/local/bin:/usr/bin:/bin",
            "bun commands on a node app put bun first"
        );
        assert_eq!(
            get("npm_config_cache").as_deref(),
            Some("/var/lib/ferrum/apps/ledger/shared/cache/npm")
        );
        assert_eq!(
            get("BUN_INSTALL_CACHE_DIR").as_deref(),
            Some("/var/lib/ferrum/apps/ledger/shared/cache/bun")
        );
        assert_eq!(
            get("HOME").as_deref(),
            Some("/var/lib/ferrum/apps/ledger/shared")
        );
        assert_eq!(get("SECRET").as_deref(), Some("hunter2"));
        assert!(
            get("DATABASE_URL")
                .unwrap()
                .starts_with("postgres://ledger_prod:")
        );
        assert_eq!(
            get("PORT").as_deref(),
            Some(health.port.to_string().as_str())
        );
        assert_eq!(
            get("NODE_ENV"),
            None,
            "production at install time drops devDependencies"
        );
        assert_eq!(build.env.iter().filter(|(k, _)| k == "PATH").count(), 1);

        let calls = p.calls();
        let link = position(&calls, |c| {
            c.starts_with("symlink_swap /var/lib/ferrum/apps/ledger/shared/cache/next ")
                && c.ends_with("/.next/cache")
        });
        let build = position(&calls, |c| {
            c.starts_with("run_scoped") && c.contains("bun run build")
        });
        let chown = position(&calls, |c| {
            c.starts_with("chown_tree /var/lib/ferrum/apps/ledger/releases/")
        });
        assert!(link < chown && chown < build, "{calls:#?}");
        assert!(
            calls.contains(
                &"make_dirs /var/lib/ferrum/apps/ledger/shared/cache/next 750".to_string()
            )
        );
    }

    async fn migrating_app(state: &State, p: &FakePlatform, port: u16, pause: bool) -> App {
        let app = provisioned(state, p, "ledger", port, |new| {
            new.commands.migrate = Some("bun run db:migrate".into());
            new.pause_for_migrations = pause;
        })
        .await;
        postgres::create(state, p, postgres::tests::new("ledger_prod"))
            .await
            .unwrap();
        postgres::link(state, &app.id, "ledger_prod").await.unwrap();
        app
    }

    #[tokio::test]
    async fn a_migration_command_snapshots_pauses_migrates_and_lifts_after_health() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let health = Health::serve(200).await;
        let app = migrating_app(&state, &p, health.port, true).await;
        let ctx = ctx(&state, &p);
        let (outcome, d) = deploy(&ctx, &app, "abc1234").await;
        assert_eq!(outcome, Outcome::Live, "{:?}", d.failure_reason);

        let calls = p.calls();
        let build = position(&calls, |c| {
            c.starts_with("run_scoped") && c.contains("bun run build")
        });
        let dump = position(&calls, |c| {
            c.starts_with("postgres_dump ledger_prod /var/lib/ferrum/snapshots/ledger_prod/")
        });
        let pause = position(&calls, |c| {
            c == "write_file /var/lib/ferrum/apps/ledger/maintenance 644"
        });
        let migrate = position(&calls, |c| {
            c.starts_with("run_scoped") && c.contains("db:migrate")
        });
        let swap = position(&calls, |c| {
            c.starts_with("symlink_swap /var/lib/ferrum/apps/ledger/releases/")
        });
        let start = position(&calls, |c| c == "service enable-now ferrum-app-ledger");
        let lift = position(&calls, |c| {
            c == "remove_file /var/lib/ferrum/apps/ledger/maintenance"
        });
        assert!(
            build < dump
                && dump < pause
                && pause < migrate
                && migrate < swap
                && swap < start
                && start < lift,
            "{calls:#?}"
        );
        assert!(
            calls.contains(&"write_file /var/lib/ferrum/pages/maintenance.html 644".to_string())
        );

        let migration = p
            .runs()
            .into_iter()
            .find(|r| r.command == "bun run db:migrate")
            .unwrap();
        assert!(
            migration
                .cwd
                .starts_with("/var/lib/ferrum/apps/ledger/releases/")
        );
        assert!(
            migration
                .env
                .iter()
                .any(|(k, v)| k == "DATABASE_URL" && v.contains("ledger_prod"))
        );
        assert!(
            migration
                .env
                .iter()
                .any(|(k, v)| k == "NODE_ENV" && v == "production")
        );
        assert_eq!(d.snapshots.len(), 1);
        assert_eq!(d.snapshots[0].database, "ledger_prod");
        let skipped: Vec<_> = d
            .steps
            .iter()
            .filter(|s| s.status != StepStatus::Done)
            .map(|s| s.state)
            .collect();
        assert_eq!(
            skipped,
            vec![DeployState::InstallingSystemPackages],
            "{:#?}",
            d.steps
        );
        assert_eq!(d.steps[5].note.as_deref(), Some("1 database(s)"));
    }

    #[tokio::test]
    async fn pause_for_migrations_off_keeps_serving() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let health = Health::serve(200).await;
        let app = migrating_app(&state, &p, health.port, false).await;
        let ctx = ctx(&state, &p);
        let (outcome, d) = deploy(&ctx, &app, "abc1234").await;
        assert_eq!(outcome, Outcome::Live);
        let calls = p.calls();
        assert!(
            !calls.iter().any(|c| c.contains("/apps/ledger/maintenance")),
            "{calls:#?}"
        );
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("postgres_dump ledger_prod"))
        );
        assert_eq!(d.steps[6].status, StepStatus::Skipped);
        assert_eq!(d.steps[6].note.as_deref(), Some("traffic kept flowing"));
        assert_eq!(d.steps[11].status, StepStatus::Skipped);
    }

    #[tokio::test]
    async fn a_failed_migration_aborts_before_the_swap_and_keeps_the_old_release() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let health = Health::serve(200).await;
        let app = migrating_app(&state, &p, health.port, true).await;
        let ctx = ctx(&state, &p);
        let (first, _) = deploy(&ctx, &app, "1111111").await;
        assert_eq!(first, Outcome::Live);
        let app = apps::by_slug(&state, "ledger").await.unwrap().unwrap();
        let old = app.current_release_id.clone().unwrap();

        p.script_run(
            "db:migrate",
            &["applying 0002", "ERROR: relation \"users\" already exists"],
            Exit::Code(1),
        );
        let before = p.calls().len();
        let (outcome, d) = deploy(&ctx, &app, "2222222").await;
        assert_eq!(outcome, Outcome::Failed);
        let reason = d.failure_reason.unwrap();
        assert!(reason.contains("status 1"), "{reason}");
        assert!(reason.contains("already exists"), "{reason}");
        let calls: Vec<String> = p.calls().into_iter().skip(before).collect();
        assert!(
            !calls
                .iter()
                .any(|c| c.starts_with("symlink_swap") && c.ends_with("/current")),
            "{calls:#?}"
        );
        assert!(calls.contains(&"remove_file /var/lib/ferrum/apps/ledger/maintenance".to_string()));
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("remove_tree /var/lib/ferrum/apps/ledger/releases/")),
            "the half-built release is removed"
        );
        let app = apps::by_slug(&state, "ledger").await.unwrap().unwrap();
        assert_eq!(app.current_release_id.as_deref(), Some(old.as_str()));
        assert_eq!(
            d.snapshots.len(),
            1,
            "the snapshot survives for the restore button"
        );
        assert_eq!(d.steps[7].status, StepStatus::Failed);
        assert_eq!(d.release_id, None);
    }

    #[tokio::test]
    async fn a_build_killed_by_its_memory_limit_names_the_limit() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        p.script_run(
            "bun run build",
            &["FATAL ERROR: heap out of memory"],
            Exit::Killed { signal: 9 },
        );
        let app = provisioned(&state, &p, "ledger", 20999, |_| {}).await;
        let before = p.calls().len();
        let ctx = ctx(&state, &p);
        let (outcome, d) = deploy(&ctx, &app, "abc1234").await;
        assert_eq!(outcome, Outcome::Failed);
        assert_eq!(
            d.failure_reason.as_deref(),
            Some(
                "The build exceeded 1200 MB and was stopped. Raise the build limit or reduce peak memory."
            )
        );
        assert!(releases::for_app(&state, &app.id).await.unwrap().is_empty());
        assert!(
            p.calls()
                .iter()
                .any(|c| c.starts_with("remove_tree /var/lib/ferrum/apps/ledger/releases/"))
        );
        assert_eq!(
            p.list_dir(Path::new("/var/lib/ferrum/apps/ledger/releases"))
                .unwrap(),
            Vec::<String>::new()
        );
        assert!(!p.calls()[before..].iter().any(|c| c.starts_with("service")));

        p.script_run("bun run build", &[], Exit::TimedOut);
        let (_, d) = deploy(&ctx, &app, "abc1235").await;
        assert_eq!(
            d.failure_reason.as_deref(),
            Some("The build did not finish within 20 minutes.")
        );
        p.script_run("bun run build", &[], Exit::Code(137));
        let (_, d) = deploy(&ctx, &app, "abc1236").await;
        assert!(d.failure_reason.unwrap().starts_with("The build exceeded"));
    }

    #[tokio::test]
    async fn failed_health_rolls_back_to_the_previous_release_and_restarts_again() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let health = Health::serve(200).await;
        let app = provisioned(&state, &p, "ledger", health.port, |_| {}).await;
        let ctx = ctx(&state, &p);
        let (first, d1) = deploy(&ctx, &app, "1111111").await;
        assert_eq!(first, Outcome::Live);
        let app = apps::by_slug(&state, "ledger").await.unwrap().unwrap();

        let broken = Health::serve(500).await;
        sqlx::query("UPDATE app_ports SET port = ? WHERE app_id = ?")
            .bind(broken.port as i64)
            .bind(&app.id)
            .execute(&state.pool)
            .await
            .unwrap();
        let app = apps::by_slug(&state, "ledger").await.unwrap().unwrap();
        let before = p.calls().len();
        let (outcome, d2) = deploy(&ctx, &app, "2222222").await;
        assert_eq!(outcome, Outcome::RolledBack);
        let reason = d2.failure_reason.unwrap();
        assert!(reason.contains("did not pass within 5s"), "{reason}");
        assert!(reason.contains("Rolled back to 1111111"), "{reason}");
        assert!(broken.hits() >= 2);

        let calls: Vec<String> = p.calls().into_iter().skip(before).collect();
        let swaps: Vec<&String> = calls
            .iter()
            .filter(|c| c.starts_with("symlink_swap") && c.ends_with("/current"))
            .collect();
        assert_eq!(swaps.len(), 2, "{calls:#?}");
        let first_release = releases::by_id(&state, d1.release_id.as_ref().unwrap())
            .await
            .unwrap()
            .unwrap();
        assert!(swaps[1].starts_with(&format!("symlink_swap {} ", first_release.dir)));
        assert_eq!(
            calls
                .iter()
                .filter(|c| *c == "service restart ferrum-app-ledger")
                .count(),
            2
        );
        let app = apps::by_slug(&state, "ledger").await.unwrap().unwrap();
        assert_eq!(app.current_release_id, d1.release_id);
        assert_eq!(d2.steps[10].status, StepStatus::Failed);
        assert!(d2.release_id.is_some(), "the built release stays on disk");
        assert_eq!(releases::for_app(&state, &app.id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn a_first_deploy_whose_health_fails_stops_the_unit() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let broken = Health::serve(503).await;
        let app = provisioned(&state, &p, "ledger", broken.port, |_| {}).await;
        let ctx = ctx(&state, &p);
        let (outcome, d) = deploy(&ctx, &app, "1111111").await;
        assert_eq!(outcome, Outcome::Failed);
        assert!(d.failure_reason.unwrap().contains("did not pass within 5s"));
        assert!(
            p.calls()
                .contains(&"service stop ferrum-app-ledger".to_string())
        );
        assert_eq!(
            apps::by_slug(&state, "ledger")
                .await
                .unwrap()
                .unwrap()
                .current_release_id,
            None
        );
    }

    #[tokio::test]
    async fn a_static_site_ends_at_swapping() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let app = provisioned(&state, &p, "docs", 20998, |new| {
            new.runtime = RuntimeKind::Static;
            new.output_dir = Some("dist".into());
        })
        .await;
        let before = p.calls().len();
        let ctx = ctx(&state, &p);
        let (outcome, d) = deploy(&ctx, &app, "abc1234").await;
        assert_eq!(outcome, Outcome::Live, "{:?}", d.failure_reason);
        assert!(!p.calls()[before..].iter().any(|c| c.starts_with("service")));
        assert_eq!(d.steps[9].status, StepStatus::Skipped);
        assert_eq!(d.steps[10].status, StepStatus::Skipped);
        assert!(p.link("/var/lib/ferrum/apps/docs/current").is_some());
    }

    #[tokio::test]
    async fn a_deploy_without_a_sha_asks_github_and_fails_cleanly_when_it_cannot() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let app = provisioned(&state, &p, "ledger", 20997, |_| {}).await;
        let ctx = ctx(&state, &p);
        let d = create(&state, &app, Trigger::Webhook, "v1.0", &Commit::default())
            .await
            .unwrap();
        assert_eq!(run(&ctx, &d.id).await.unwrap(), Outcome::Failed);
        let d = by_id(&state, &d.id).await.unwrap().unwrap();
        assert!(d.failure_reason.unwrap().contains("GitHub"));
        assert!(!p.calls().iter().any(|c| c.starts_with("git_clone")));
        assert_eq!(d.steps[1].status, StepStatus::Failed);
    }

    #[tokio::test]
    async fn releases_are_pruned_to_five_and_the_current_link_follows_the_newest() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let health = Health::serve(200).await;
        let app = provisioned(&state, &p, "ledger", health.port, |_| {}).await;
        let ctx = ctx(&state, &p);
        for i in 0..7 {
            let (outcome, _) = deploy(&ctx, &app, &format!("{i}{i}{i}{i}{i}{i}{i}")).await;
            assert_eq!(outcome, Outcome::Live);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let on_disk = p
            .list_dir(Path::new("/var/lib/ferrum/apps/ledger/releases"))
            .unwrap();
        assert_eq!(on_disk.len(), 5, "{on_disk:?}");
        assert!(
            on_disk
                .iter()
                .all(|d| !d.ends_with("_0000000") && !d.ends_with("_1111111"))
        );
        assert_eq!(releases::for_app(&state, &app.id).await.unwrap().len(), 5);
        assert_eq!(
            p.calls()
                .iter()
                .filter(|c| *c == "service restart ferrum-app-ledger")
                .count(),
            6
        );
    }

    #[tokio::test]
    async fn a_deploy_refuses_to_start_below_the_disk_threshold() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        p.set_disk_free(200 * 1024 * 1024);
        let app = provisioned(&state, &p, "ledger", 20996, |_| {}).await;
        let ctx = ctx(&state, &p);
        let (outcome, d) = deploy(&ctx, &app, "abc1234").await;
        assert_eq!(outcome, Outcome::Failed);
        let reason = d.failure_reason.unwrap();
        assert!(reason.contains("200 MB"), "{reason}");
        assert!(reason.contains("1024 MB"), "{reason}");
        assert!(!p.calls().iter().any(|c| c.starts_with("git_clone")));
    }

    #[tokio::test]
    async fn system_packages_are_installed_before_dependencies_and_git_when_missing() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let health = Health::serve(200).await;
        let app = provisioned(&state, &p, "ledger", health.port, |new| {
            new.packages = vec!["ffmpeg".into()];
        })
        .await;
        let ctx = ctx(&state, &p);
        let (outcome, d) = deploy(&ctx, &app, "abc1234").await;
        assert_eq!(outcome, Outcome::Live);
        let calls = p.calls();
        let git = position(&calls, |c| c == "install_packages git");
        let clone = position(&calls, |c| c.starts_with("git_clone"));
        let ffmpeg = position(&calls, |c| c == "install_packages ffmpeg");
        let install = position(&calls, |c| {
            c.starts_with("run_scoped") && c.contains("bun install")
        });
        assert!(
            git < clone && clone < ffmpeg && ffmpeg < install,
            "{calls:#?}"
        );
        assert_eq!(d.steps[2].status, StepStatus::Done);
        assert_eq!(d.steps[2].note.as_deref(), Some("1 package"));

        p.write_file(Path::new("/usr/bin/git"), "", 0o755).unwrap();
        let before = p.calls().len();
        deploy(&ctx, &app, "abc1235").await;
        assert!(!p.calls()[before..].contains(&"install_packages git".to_string()));
    }

    #[tokio::test]
    async fn a_rollback_repoints_without_a_build_and_can_restore_the_snapshot() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let health = Health::serve(200).await;
        let app = migrating_app(&state, &p, health.port, true).await;
        let ctx = ctx(&state, &p);
        let (_, d1) = deploy(&ctx, &app, "1111111").await;
        let (_, d2) = deploy(&ctx, &app, "2222222").await;
        assert_eq!(d2.outcome, Some(Outcome::Live));
        let first = releases::by_id(&state, d1.release_id.as_ref().unwrap())
            .await
            .unwrap()
            .unwrap();
        let app = apps::by_slug(&state, "ledger").await.unwrap().unwrap();

        let before = p.calls().len();
        let rollback = create(
            &state,
            &app,
            Trigger::Rollback,
            &first.git_ref,
            &commit("1111111"),
        )
        .await
        .unwrap();
        crate::deploy::set_rollback_target(&state, &rollback.id, &first.id, Some(&d2.id))
            .await
            .unwrap();
        assert_eq!(run(&ctx, &rollback.id).await.unwrap(), Outcome::Live);
        let calls: Vec<String> = p.calls().into_iter().skip(before).collect();
        assert!(
            !calls
                .iter()
                .any(|c| c.starts_with("run_scoped") || c.starts_with("git_clone")),
            "{calls:#?}"
        );
        let restore = position(&calls, |c| c.starts_with("postgres_restore ledger_prod"));
        let pause = position(&calls, |c| {
            c == "write_file /var/lib/ferrum/apps/ledger/maintenance 644"
        });
        let swap = position(&calls, |c| {
            c == format!(
                "symlink_swap {} /var/lib/ferrum/apps/ledger/current",
                first.dir
            )
        });
        let restart = position(&calls, |c| c == "service restart ferrum-app-ledger");
        let lift = position(&calls, |c| {
            c == "remove_file /var/lib/ferrum/apps/ledger/maintenance"
        });
        assert!(
            pause < restore && restore < swap && swap < restart && restart < lift,
            "{calls:#?}"
        );
        let d = by_id(&state, &rollback.id).await.unwrap().unwrap();
        assert_eq!(d.release_id.as_deref(), Some(first.id.as_str()));
        assert_eq!(d.steps[1].status, StepStatus::Skipped);
        assert_eq!(d.steps[1].note.as_deref(), Some("rolling back to 1111111"));
        assert_eq!(d.steps[7].note.as_deref(), Some("restored 1 snapshot(s)"));
        assert_eq!(
            apps::by_slug(&state, "ledger")
                .await
                .unwrap()
                .unwrap()
                .current_release_id,
            Some(first.id.clone())
        );
        assert_eq!(
            releases::for_app(&state, &app.id).await.unwrap().len(),
            2,
            "a rollback records no new release"
        );
    }
}
