use crate::apps::App;
use crate::apps::provision::user_name;
use crate::deploy::releases::{current_link, on_disk};
use crate::deploy::run::{CPU_WEIGHT, Ctx, IO_WEIGHT};
use crate::deploy::steps::{command_env, exit_sentence, work_dir};
use crate::deploy::{self, log, short};
use crate::runtime::Phase;
use crate::state::State;
use crate::time;
use ferrum_platform::{RunSpec, Stream};
use serde::Serialize;

pub const HISTORY: i64 = 20;
pub const OK: &str = "ok";
const INTERRUPTED: &str = "Interrupted by a Ferrum restart.";

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("Type a command to run.")]
    Blank,
    #[error("Deploy the application first; commands run in its current release.")]
    NotDeployed,
    #[error("A deploy is running for that application; wait for it to finish.")]
    DeployRunning,
    #[error("A command is still running for that application; wait for it to finish.")]
    RunOpen,
}

#[derive(Debug, Clone, Serialize)]
pub struct Run {
    pub id: String,
    pub app_id: String,
    pub command: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub exit: Option<String>,
}

pub async fn by_id(state: &State, id: &str) -> anyhow::Result<Option<Run>> {
    let row = sqlx::query!(
        r#"SELECT id AS "id!", app_id AS "app_id!", command AS "command!",
                  started_at AS "started_at!", finished_at, exit
           FROM command_runs WHERE id = ?"#,
        id
    )
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|r| Run {
        id: r.id,
        app_id: r.app_id,
        command: r.command,
        started_at: time::utc(r.started_at),
        finished_at: r.finished_at.map(time::utc),
        exit: r.exit,
    }))
}

pub async fn list(state: &State, app_id: &str) -> anyhow::Result<Vec<Run>> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", app_id AS "app_id!", command AS "command!",
                  started_at AS "started_at!", finished_at, exit
           FROM command_runs WHERE app_id = ? ORDER BY started_at DESC, rowid DESC LIMIT ?"#,
        app_id,
        HISTORY
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Run {
            id: r.id,
            app_id: r.app_id,
            command: r.command,
            started_at: time::utc(r.started_at),
            finished_at: r.finished_at.map(time::utc),
            exit: r.exit,
        })
        .collect())
}

async fn open_for(state: &State, app_id: &str) -> anyhow::Result<bool> {
    let open: i64 = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM command_runs WHERE app_id = ? AND finished_at IS NULL"#,
        app_id
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(open > 0)
}

/// Runs left open by a daemon that stopped mid-command are closed at the next start.
pub async fn close_interrupted(state: &State) -> anyhow::Result<u64> {
    let done = sqlx::query!(
        "UPDATE command_runs SET finished_at = datetime('now'), exit = ? WHERE finished_at IS NULL",
        INTERRUPTED
    )
    .execute(&state.pool)
    .await?;
    Ok(done.rows_affected())
}

pub async fn start(ctx: &Ctx, app: &App, command: &str) -> anyhow::Result<Run> {
    let command = command.trim();
    if command.is_empty() {
        return Err(CommandError::Blank.into());
    }
    let current = current_link(app);
    if !on_disk(ctx.platform.as_ref(), &current) {
        return Err(CommandError::NotDeployed.into());
    }
    if deploy::running_for(&ctx.state, &app.id).await?.is_some() {
        return Err(CommandError::DeployRunning.into());
    }
    if open_for(&ctx.state, &app.id).await? {
        return Err(CommandError::RunOpen.into());
    }
    let ctx = ctx.with_current_limits().await?;
    let id = uuid::Uuid::new_v4().to_string();
    let spec = RunSpec {
        unit: format!("ferrum-run-{}-{}", app.slug, short(&id)),
        user: user_name(&app.slug),
        cwd: work_dir(&current, &app.root),
        command: command.to_string(),
        env: command_env(&ctx, app, Phase::Run).await?,
        memory_max_mb: ctx.build_memory_mb,
        cpu_weight: CPU_WEIGHT,
        io_weight: IO_WEIGHT,
        timeout: ctx.migrate_timeout,
    };
    sqlx::query!(
        "INSERT INTO command_runs (id, app_id, command) VALUES (?, ?, ?)",
        id,
        app.id,
        command
    )
    .execute(&ctx.state.pool)
    .await?;
    let run = by_id(&ctx.state, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the run vanished"))?;
    tokio::spawn(async move {
        let exit = match execute(&ctx, &id, spec).await {
            Ok(exit) => exit,
            Err(e) => {
                tracing::error!(error = ?e, run = %id, "running a command");
                format!("Ferrum could not run the command: {e:#}")
            }
        };
        if let Err(e) = finish(&ctx, &id, &exit).await {
            tracing::error!(error = ?e, run = %id, "recording a command's exit");
        }
    });
    Ok(run)
}

async fn execute(ctx: &Ctx, id: &str, spec: RunSpec) -> anyhow::Result<String> {
    log::append_run(
        &ctx.state,
        &ctx.log,
        id,
        "system",
        &format!("$ {}", spec.command),
    )
    .await?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Stream, String)>();
    let platform = ctx.platform.clone();
    let spawned = spec.clone();
    let handle = tokio::task::spawn_blocking(move || {
        platform.run_scoped(&spawned, &mut |stream, line| {
            let _ = tx.send((stream, line.to_string()));
        })
    });
    while let Some((stream, line)) = rx.recv().await {
        let name = match stream {
            Stream::Stdout => "stdout",
            Stream::Stderr => "stderr",
        };
        log::append_run(&ctx.state, &ctx.log, id, name, &line).await?;
    }
    let exit = handle.await??;
    Ok(exit_sentence("command", &exit, spec.memory_max_mb, spec.timeout).unwrap_or(OK.into()))
}

async fn finish(ctx: &Ctx, id: &str, exit: &str) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE command_runs SET finished_at = datetime('now'), exit = ? WHERE id = ?",
        exit,
        id
    )
    .execute(&ctx.state.pool)
    .await?;
    ctx.log.run_done(id, exit);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::apps::{self, env};
    use crate::deploy::log::Event;
    use crate::github::Api;
    use crate::runtime::toolchain::Store;
    use ferrum_platform::{Exit, FakePlatform, Platform};
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    fn ctx(state: &State, platform: &Arc<FakePlatform>) -> Ctx {
        Ctx::new(
            state.clone(),
            platform.clone(),
            Api::at("http://127.0.0.1:1").with_fixed_token("ghs_fixed"),
            crate::http::client(),
            Store::at("/var/lib/ferrum/runtimes"),
        )
    }

    async fn deployed(state: &State, platform: &FakePlatform) -> App {
        let app = apps::create(state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        platform
            .write_file(
                Path::new("/var/lib/ferrum/apps/ledger/current/.git/HEAD"),
                "ref: refs/heads/main\n",
                0o644,
            )
            .unwrap();
        app
    }

    async fn settled(state: &State, id: &str) -> Run {
        for _ in 0..200 {
            let run = by_id(state, id).await.unwrap().unwrap();
            if run.finished_at.is_some() {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the run never finished");
    }

    #[tokio::test]
    async fn a_command_runs_as_the_app_in_its_current_release_with_its_environment() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let app = deployed(&state, &p).await;
        env::set(&state, &app.id, "ADMIN_EMAIL", "root@example.com")
            .await
            .unwrap();
        p.script_run("seed:admin", &["seeding", "done"], Exit::Code(0));
        let ctx = ctx(&state, &p);
        let mut live = ctx.log.subscribe();
        let run = start(&ctx, &app, "  bun run seed:admin 'pw'  ")
            .await
            .unwrap();
        assert_eq!(run.command, "bun run seed:admin 'pw'");
        assert!(run.finished_at.is_none());
        let done = settled(&state, &run.id).await;
        assert_eq!(done.exit.as_deref(), Some(OK));

        let spec = &p.runs()[0];
        assert_eq!(spec.user, "ferrum-ledger");
        assert_eq!(spec.cwd, Path::new("/var/lib/ferrum/apps/ledger/current"));
        assert!(spec.unit.starts_with("ferrum-run-ledger-"));
        assert_eq!(spec.timeout, ctx.migrate_timeout);
        assert_eq!(spec.memory_max_mb, ctx.build_memory_mb);
        let var = |k: &str| {
            spec.env
                .iter()
                .find(|(key, _)| key == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(var("ADMIN_EMAIL").as_deref(), Some("root@example.com"));
        assert_eq!(
            var("PORT").as_deref(),
            Some(&app.routes[0].port.to_string()[..])
        );
        assert_eq!(
            var("HOME").as_deref(),
            Some("/var/lib/ferrum/apps/ledger/shared")
        );

        let lines = log::run_lines(&state, &run.id, 0).await.unwrap();
        let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["$ bun run seed:admin 'pw'", "seeding", "done"]);
        assert_eq!(lines[0].stream, "system");
        assert_eq!(lines[1].stream, "stdout");
        assert!(matches!(live.recv().await.unwrap(), Event::RunLine { .. }));
        assert_eq!(list(&state, &app.id).await.unwrap()[0].id, run.id);
    }

    #[tokio::test]
    async fn a_failing_command_records_the_reason() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let app = deployed(&state, &p).await;
        let ctx = ctx(&state, &p);
        p.script_run("false", &["boom"], Exit::Code(2));
        let run = start(&ctx, &app, "false").await.unwrap();
        assert_eq!(
            settled(&state, &run.id).await.exit.as_deref(),
            Some("The command exited with status 2")
        );
        p.script_run("hog", &[], Exit::Killed { signal: 9 });
        let run = start(&ctx, &app, "hog").await.unwrap();
        assert!(
            settled(&state, &run.id)
                .await
                .exit
                .unwrap()
                .contains("exceeded")
        );
        p.script_run("sleep", &[], Exit::TimedOut);
        let run = start(&ctx, &app, "sleep 999").await.unwrap();
        assert_eq!(
            settled(&state, &run.id).await.exit.as_deref(),
            Some("The command did not finish within 10 minutes.")
        );
        let runs = list(&state, &app.id).await.unwrap();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].command, "sleep 999");
    }

    #[tokio::test]
    async fn a_command_is_refused_before_a_deploy_during_one_and_beside_another() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let ctx = ctx(&state, &p);
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let e = start(&ctx, &app, "ls").await.unwrap_err();
        assert!(matches!(
            e.downcast_ref::<CommandError>(),
            Some(CommandError::NotDeployed)
        ));
        let e = start(&ctx, &app, "   ").await.unwrap_err();
        assert!(matches!(
            e.downcast_ref::<CommandError>(),
            Some(CommandError::Blank)
        ));

        p.write_file(
            Path::new("/var/lib/ferrum/apps/ledger/current/.git/HEAD"),
            "ref: refs/heads/main\n",
            0o644,
        )
        .unwrap();
        let gate = p.gate("wait");
        let first = start(&ctx, &app, "wait").await.unwrap();
        let e = start(&ctx, &app, "ls").await.unwrap_err();
        assert!(matches!(
            e.downcast_ref::<CommandError>(),
            Some(CommandError::RunOpen)
        ));
        gate.open();
        settled(&state, &first.id).await;

        let d = deploy::create(
            &state,
            &app,
            deploy::Trigger::Manual,
            "main",
            &deploy::Commit::default(),
        )
        .await
        .unwrap();
        deploy::enter(&state, &d.id, deploy::DeployState::Cloning)
            .await
            .unwrap();
        let e = start(&ctx, &app, "ls").await.unwrap_err();
        assert!(matches!(
            e.downcast_ref::<CommandError>(),
            Some(CommandError::DeployRunning)
        ));
    }

    #[tokio::test]
    async fn a_restart_closes_the_runs_it_interrupted() {
        let (_d, state) = state().await;
        let p = Arc::new(FakePlatform::new());
        let app = deployed(&state, &p).await;
        let ctx = ctx(&state, &p);
        let gate = p.gate("wait");
        let run = start(&ctx, &app, "wait").await.unwrap();
        assert_eq!(close_interrupted(&state).await.unwrap(), 1);
        let closed = by_id(&state, &run.id).await.unwrap().unwrap();
        assert_eq!(closed.exit.as_deref(), Some(INTERRUPTED));
        gate.open();
        assert_eq!(close_interrupted(&state).await.unwrap(), 0);
    }
}
