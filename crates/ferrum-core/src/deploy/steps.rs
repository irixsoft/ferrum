use super::releases::{self, Release};
use super::run::{CPU_WEIGHT, Ctx, DISK_MIN_BYTES, IO_WEIGHT};
use super::{Commit, Deploy, DeployState, Outcome, Trigger, log, maintenance, short, snapshots};
use crate::apps::provision::{app_dir, user_name, write_env};
use crate::apps::unit::unit_name;
use crate::apps::{App, env};
use crate::github::commits;
use crate::runtime::toolchain::{self, Store};
use crate::runtime::{self, Phase, RuntimeKind};
use crate::{postgres, runtime as rt};
use anyhow::{Context, bail};
use ferrum_platform::ubuntu::GIT;
use ferrum_platform::{Exit, RunSpec, ServiceAction, Stream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SYSTEM: &str = "system";
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);

pub struct Job {
    ctx: Ctx,
    app: App,
    deploy: Deploy,
    release_dir: Option<PathBuf>,
    swapped: bool,
    maintenance_on: bool,
    previous: Option<Release>,
}

impl Job {
    pub fn new(ctx: Ctx, app: App, deploy: Deploy) -> Self {
        Self {
            ctx,
            app,
            deploy,
            release_dir: None,
            swapped: false,
            maintenance_on: false,
            previous: None,
        }
    }

    pub async fn pipeline(&mut self) -> anyhow::Result<Outcome> {
        if self.deploy.trigger == Trigger::Rollback {
            return self.rollback().await;
        }
        self.clone_step().await?;
        self.packages_step().await?;
        self.command_step(DeployState::InstallingDeps, "install")
            .await?;
        self.command_step(DeployState::Building, "build").await?;
        self.migration_steps().await?;
        let dir = self.release_dir.clone().expect("cloned before swapping");
        let release = releases::record(
            &self.ctx.state,
            &self.app,
            &dir,
            &self.deploy.git_ref,
            self.deploy.commit_sha.as_deref().unwrap_or_default(),
            self.deploy.commit_message.as_deref(),
        )
        .await?;
        self.tail(release).await
    }

    async fn rollback(&mut self) -> anyhow::Result<Outcome> {
        let target_id = self
            .deploy
            .release_id
            .clone()
            .context("the rollback names no release")?;
        let target = releases::by_id(&self.ctx.state, &target_id)
            .await?
            .context("That release no longer exists.")?;
        if !releases::on_disk(self.ctx.platform.as_ref(), Path::new(&target.dir)) {
            bail!(
                "Release {} is no longer on disk; deploy {} again instead.",
                short(&target.commit_sha),
                target.git_ref
            );
        }
        let note = format!("rolling back to {}", short(&target.commit_sha));
        self.say(&format!(
            "Rolling back to {} ({})",
            short(&target.commit_sha),
            target.git_ref
        ))
        .await?;
        for state in [
            DeployState::Cloning,
            DeployState::InstallingSystemPackages,
            DeployState::InstallingDeps,
            DeployState::Building,
            DeployState::Snapshotting,
        ] {
            self.skip(state, &note).await?;
        }
        let restore = match &self.deploy.restore_deploy_id {
            Some(deploy_id) => snapshots::for_deploy(&self.ctx.state, deploy_id).await?,
            None => Vec::new(),
        };
        if restore.is_empty() {
            self.skip(DeployState::MaintenanceOn, "no snapshot to restore")
                .await?;
            self.skip(DeployState::Migrating, "no snapshot to restore")
                .await?;
        } else {
            self.maintenance_on_step().await?;
            self.enter(DeployState::Migrating).await?;
            for snapshot in &restore {
                self.say(&format!(
                    "Restoring {} from the snapshot taken at {}",
                    snapshot.database, snapshot.taken_at
                ))
                .await?;
                snapshots::restore(&self.ctx.state, self.ctx.platform.as_ref(), &snapshot.id)
                    .await
                    .with_context(|| format!("restoring {}", snapshot.database))?;
            }
            super::note(
                &self.ctx.state,
                &self.deploy.id,
                DeployState::Migrating,
                &format!("restored {} snapshot(s)", restore.len()),
            )
            .await?;
        }
        self.tail(target).await
    }

    /// Swap, restart, health, lift maintenance; shared by a build and a rollback.
    async fn tail(&mut self, release: Release) -> anyhow::Result<Outcome> {
        self.swap_step(&release).await?;
        if let Err(reason) = self.restart_step().await {
            return self.recover(format!("{reason:#}"), &release).await;
        }
        if let Some(reason) = self.health_step().await? {
            return self.recover(reason, &release).await;
        }
        self.maintenance_off_step().await?;
        super::finish(
            &self.ctx.state,
            &self.deploy.id,
            Outcome::Live,
            None,
            Some(&release.id),
        )
        .await?;
        self.say(&format!("Live at {}", short(&release.commit_sha)))
            .await?;
        let protect: Vec<&str> = self.previous.iter().map(|r| r.id.as_str()).collect();
        releases::prune(
            &self.ctx.state,
            self.ctx.platform.as_ref(),
            &self.app,
            releases::KEEP,
            &protect,
        )
        .await?;
        Ok(Outcome::Live)
    }

    pub async fn abort(&mut self, error: anyhow::Error) -> anyhow::Result<Outcome> {
        let reason = format!("{error:#}");
        let _ = self.say(&format!("✗ {reason}")).await;
        self.lift_maintenance().await;
        if let Some(dir) = &self.release_dir
            && !self.swapped
        {
            let _ = self.ctx.platform.remove_tree(dir);
        }
        super::finish(
            &self.ctx.state,
            &self.deploy.id,
            Outcome::Failed,
            Some(&reason),
            None,
        )
        .await?;
        Ok(Outcome::Failed)
    }

    async fn clone_step(&mut self) -> anyhow::Result<()> {
        self.enter(DeployState::Cloning).await?;
        let platform = self.ctx.platform.clone();
        let free = platform.disk_free_bytes(&app_dir(&self.app.slug))?;
        if free < DISK_MIN_BYTES {
            bail!(
                "Only {} MB of disk is free and a deploy needs at least {} MB. Free some space and try again.",
                free / (1024 * 1024),
                DISK_MIN_BYTES / (1024 * 1024)
            );
        }
        if !platform.file_exists(Path::new(GIT)) {
            self.say("Installing git").await?;
            platform
                .install_packages(&["git"])
                .context("installing git")?;
        }
        if self.deploy.commit_sha.is_none() {
            let head = commits::head_of(
                &self.ctx.github,
                &self.ctx.state,
                &self.app,
                Some(&self.deploy.git_ref),
            )
            .await?;
            let commit = Commit {
                sha: Some(head.sha),
                message: Some(head.message),
                author: Some(head.author),
            };
            super::set_commit(&self.ctx.state, &self.deploy.id, &commit).await?;
            self.deploy.commit_sha = commit.sha;
            self.deploy.commit_message = commit.message;
            self.deploy.author = commit.author;
        }
        let sha = self.deploy.commit_sha.clone().expect("resolved above");
        let repository = &self.app.repository;
        let token = self.ctx.github.installation_token(&self.ctx.state).await?;
        let url = format!("https://x-access-token:{token}@github.com/{repository}.git");
        let public = format!("https://github.com/{repository}.git");
        let dir = releases::release_dir(&self.app, &sha);
        self.release_dir = Some(dir.clone());
        self.say(&format!("Cloning {repository} at {}", self.deploy.git_ref))
            .await?;
        let clone_ref =
            (!looks_like_sha(&self.deploy.git_ref)).then_some(self.deploy.git_ref.as_str());
        platform
            .git_clone(&url, clone_ref, &dir, 1)
            .context("cloning the repository")?;
        platform
            .git_checkout(&dir, &sha)
            .with_context(|| format!("checking out {}", short(&sha)))?;
        platform.git_scrub_remote(&dir, &public)?;
        let head = platform.git_head(&dir)?;
        if head != sha {
            super::set_commit(
                &self.ctx.state,
                &self.deploy.id,
                &Commit {
                    sha: Some(head.clone()),
                    ..Commit::default()
                },
            )
            .await?;
            self.deploy.commit_sha = Some(head.clone());
        }
        self.prepare_caches(&dir)?;
        let user = user_name(&self.app.slug);
        platform.chown_tree(&dir, &user)?;
        platform.chown_tree(&self.shared().join("cache"), &user)?;
        write_env(&self.ctx.state, platform.as_ref(), &self.app).await?;
        self.say(&format!("Checked out {}", short(&head))).await?;
        Ok(())
    }

    /// Framework caches live under `shared/` and are reached through a link, because the release
    /// is read-only to the running unit.
    fn prepare_caches(&self, dir: &Path) -> anyhow::Result<()> {
        let platform = &self.ctx.platform;
        let cache = self.shared().join("cache");
        for name in ["npm", "bun", "pnpm", "yarn", "nuget", "next"] {
            platform.make_dirs(&cache.join(name), 0o750)?;
        }
        if self.app.toolchain != RuntimeKind::Dotnet {
            let work = work_dir(dir, &self.app.root);
            platform.make_dirs(&work.join(".next"), 0o755)?;
            platform.symlink_swap(&cache.join("next"), &work.join(".next/cache"))?;
        }
        Ok(())
    }

    async fn packages_step(&mut self) -> anyhow::Result<()> {
        if self.app.packages.is_empty() {
            return self
                .skip(DeployState::InstallingSystemPackages, "no system packages")
                .await;
        }
        self.enter(DeployState::InstallingSystemPackages).await?;
        let resolved: Vec<String> = self
            .app
            .packages
            .iter()
            .flat_map(|p| self.ctx.platform.resolve_package(p))
            .collect();
        let names: Vec<&str> = resolved.iter().map(String::as_str).collect();
        self.say(&format!("Installing {}", names.join(", ")))
            .await?;
        self.ctx
            .platform
            .install_packages(&names)
            .context("installing system packages")?;
        let count = self.app.packages.len();
        let note = if count == 1 {
            "1 package".to_string()
        } else {
            format!("{count} packages")
        };
        super::note(
            &self.ctx.state,
            &self.deploy.id,
            DeployState::InstallingSystemPackages,
            &note,
        )
        .await
    }

    async fn command_step(&mut self, state: DeployState, what: &str) -> anyhow::Result<()> {
        let command = match what {
            "install" => self.app.commands.install.clone(),
            _ => self.app.commands.build.clone(),
        };
        let Some(command) = command.filter(|c| !c.trim().is_empty()) else {
            return self.skip(state, &format!("no {what} command")).await;
        };
        self.enter(state).await?;
        self.run_command(what, Phase::Build, &command, self.ctx.build_timeout)
            .await
    }

    async fn migration_steps(&mut self) -> anyhow::Result<()> {
        let Some(migrate) = self
            .app
            .commands
            .migrate
            .clone()
            .filter(|c| !c.trim().is_empty())
        else {
            for state in [
                DeployState::Snapshotting,
                DeployState::MaintenanceOn,
                DeployState::Migrating,
            ] {
                self.skip(state, "no migration command").await?;
            }
            return Ok(());
        };
        let linked = postgres::linked_to(&self.ctx.state, &self.app.id).await?;
        if linked.is_empty() {
            self.skip(DeployState::Snapshotting, "no linked database")
                .await?;
        } else {
            self.enter(DeployState::Snapshotting).await?;
            let taken = snapshots::take(
                &self.ctx.state,
                self.ctx.platform.as_ref(),
                &self.app,
                &self.deploy.id,
            )
            .await
            .context("taking the pre-migration snapshot")?;
            for snapshot in &taken {
                self.say(&format!(
                    "Snapshot of {} at {}",
                    snapshot.database, snapshot.path
                ))
                .await?;
            }
            super::note(
                &self.ctx.state,
                &self.deploy.id,
                DeployState::Snapshotting,
                &format!("{} database(s)", taken.len()),
            )
            .await?;
        }
        self.maintenance_on_step().await?;
        self.enter(DeployState::Migrating).await?;
        self.run_command("migration", Phase::Run, &migrate, self.ctx.migrate_timeout)
            .await
    }

    async fn maintenance_on_step(&mut self) -> anyhow::Result<()> {
        if !self.app.pause_for_migrations {
            return self
                .skip(DeployState::MaintenanceOn, "traffic kept flowing")
                .await;
        }
        self.enter(DeployState::MaintenanceOn).await?;
        maintenance::on(self.ctx.platform.as_ref(), &self.app.slug)?;
        self.maintenance_on = true;
        Ok(())
    }

    async fn swap_step(&mut self, release: &Release) -> anyhow::Result<()> {
        self.enter(DeployState::Swapping).await?;
        self.previous = match &self.app.current_release_id {
            Some(id) => releases::by_id(&self.ctx.state, id).await?,
            None => None,
        };
        self.ctx
            .platform
            .symlink_swap(Path::new(&release.dir), &releases::current_link(&self.app))?;
        releases::set_current(&self.ctx.state, &self.app.id, Some(&release.id)).await?;
        self.swapped = true;
        Ok(())
    }

    async fn restart_step(&mut self) -> anyhow::Result<()> {
        if !self.app.runtime.has_process() {
            return self
                .skip(DeployState::Restarting, "static site, nothing to restart")
                .await;
        }
        self.enter(DeployState::Restarting).await?;
        let action = if self.previous.is_some() {
            ServiceAction::Restart
        } else {
            ServiceAction::EnableNow
        };
        self.ctx
            .platform
            .service(action, &unit_name(&self.app.slug))
            .context("starting the unit")?;
        Ok(())
    }

    /// `None` when healthy; otherwise the sentence for the deploy.
    async fn health_step(&mut self) -> anyhow::Result<Option<String>> {
        if !self.app.runtime.has_process() {
            self.skip(DeployState::HealthChecking, "static site, nothing to check")
                .await?;
            return Ok(None);
        }
        self.enter(DeployState::HealthChecking).await?;
        let port = self.app.main_port().context("the app has no port")?;
        let url = format!("http://127.0.0.1:{port}{}", self.app.health.path);
        let budget = Duration::from_secs(self.app.health.startup_budget_secs as u64);
        let started = Instant::now();
        loop {
            let answer = self.ctx.http.get(&url).timeout(HEALTH_TIMEOUT).send().await;
            if let Ok(res) = answer
                && (res.status().is_success() || res.status().is_redirection())
            {
                self.say(&format!(
                    "Healthy after {}s ({} answered {})",
                    started.elapsed().as_secs(),
                    self.app.health.path,
                    res.status().as_u16()
                ))
                .await?;
                return Ok(None);
            }
            if started.elapsed() >= budget {
                return Ok(Some(format!(
                    "The health check at {} did not pass within {}s.",
                    self.app.health.path, self.app.health.startup_budget_secs
                )));
            }
            tokio::time::sleep(self.ctx.health_interval).await;
        }
    }

    /// The previous release is present and built, so repointing and restarting is enough.
    async fn recover(&mut self, reason: String, attempted: &Release) -> anyhow::Result<Outcome> {
        self.say(&format!("✗ {reason}")).await?;
        let unit = unit_name(&self.app.slug);
        let previous = self
            .previous
            .clone()
            .filter(|p| releases::on_disk(self.ctx.platform.as_ref(), Path::new(&p.dir)));
        let (outcome, reason) = match previous {
            Some(previous) => {
                self.ctx
                    .platform
                    .symlink_swap(Path::new(&previous.dir), &releases::current_link(&self.app))?;
                releases::set_current(&self.ctx.state, &self.app.id, Some(&previous.id)).await?;
                self.ctx.platform.service(ServiceAction::Restart, &unit)?;
                self.say(&format!("Rolled back to {}", short(&previous.commit_sha)))
                    .await?;
                (
                    Outcome::RolledBack,
                    format!("{reason} Rolled back to {}.", short(&previous.commit_sha)),
                )
            }
            None => {
                let _ = self.ctx.platform.service(ServiceAction::Stop, &unit);
                releases::set_current(&self.ctx.state, &self.app.id, None).await?;
                self.say("No earlier release to fall back to; the unit is stopped")
                    .await?;
                (Outcome::Failed, reason)
            }
        };
        self.lift_maintenance().await;
        super::finish(
            &self.ctx.state,
            &self.deploy.id,
            outcome,
            Some(&reason),
            Some(&attempted.id),
        )
        .await?;
        Ok(outcome)
    }

    async fn maintenance_off_step(&mut self) -> anyhow::Result<()> {
        if !self.maintenance_on {
            let note = if self
                .app
                .commands
                .migrate
                .as_deref()
                .unwrap_or("")
                .trim()
                .is_empty()
                && self.deploy.trigger != Trigger::Rollback
            {
                "no migration command"
            } else {
                "traffic was not paused"
            };
            return self.skip(DeployState::MaintenanceOff, note).await;
        }
        self.enter(DeployState::MaintenanceOff).await?;
        maintenance::off(self.ctx.platform.as_ref(), &self.app.slug)?;
        self.maintenance_on = false;
        Ok(())
    }

    async fn lift_maintenance(&mut self) {
        if self.maintenance_on {
            let _ = maintenance::off(self.ctx.platform.as_ref(), &self.app.slug);
            self.maintenance_on = false;
        }
    }

    async fn run_command(
        &self,
        what: &str,
        phase: Phase,
        command: &str,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let dir = self.release_dir.clone().expect("cloned before any command");
        let spec = RunSpec {
            unit: format!("ferrum-build-{}-{}", self.app.slug, short(&self.deploy.id)),
            user: user_name(&self.app.slug),
            cwd: work_dir(&dir, &self.app.root),
            command: command.to_string(),
            env: self.command_env(phase).await?,
            memory_max_mb: self.ctx.build_memory_mb,
            cpu_weight: CPU_WEIGHT,
            io_weight: IO_WEIGHT,
            timeout,
        };
        self.say(&format!("$ {command}")).await?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(Stream, String)>();
        let platform = self.ctx.platform.clone();
        let spawned = spec.clone();
        let handle = tokio::task::spawn_blocking(move || {
            platform.run_scoped(&spawned, &mut |stream, line| {
                let _ = tx.send((stream, line.to_string()));
            })
        });
        let mut last_stderr = None;
        let mut last = None;
        while let Some((stream, line)) = rx.recv().await {
            let name = match stream {
                Stream::Stdout => "stdout",
                Stream::Stderr => "stderr",
            };
            log::append(&self.ctx.state, &self.ctx.log, &self.deploy.id, name, &line).await?;
            if !line.trim().is_empty() {
                if stream == Stream::Stderr {
                    last_stderr = Some(line.clone());
                }
                last = Some(line);
            }
        }
        let exit = handle.await??;
        match exit {
            Exit::Code(0) => Ok(()),
            Exit::Killed { signal: 9 } | Exit::Code(137) => bail!(
                "The {what} exceeded {} MB and was stopped. Raise the build limit or reduce peak memory.",
                self.ctx.build_memory_mb
            ),
            Exit::TimedOut => bail!(
                "The {what} did not finish within {} minutes.",
                timeout.as_secs() / 60
            ),
            Exit::Killed { signal } => bail!("The {what} was killed by signal {signal}."),
            Exit::Code(code) => {
                let tail = last_stderr
                    .or(last)
                    .map(|l| format!(": {l}"))
                    .unwrap_or_default();
                bail!("The {what} exited with status {code}{tail}")
            }
        }
    }

    /// The same content as `shared/.env`, so the build sees what the unit will, plus the
    /// toolchain, a writable home and the caches.
    async fn command_env(&self, phase: Phase) -> anyhow::Result<Vec<(String, String)>> {
        let kind = match phase {
            Phase::Build => self.app.toolchain,
            Phase::Run if self.app.runtime.has_process() => self.app.runtime,
            Phase::Run => self.app.toolchain,
        };
        let toolchain_dir = self
            .ctx
            .toolchains
            .dir(self.app.toolchain, &self.app.runtime_version);
        let mut env = rt::by_kind(kind).env_for(phase, &toolchain_dir, self.app.main_port());
        if let Some(extra) = self.extra_toolchain().await?
            && let Some(path) = env.iter_mut().find(|(k, _)| k == "PATH")
        {
            path.1 = format!("{}:{}", extra.display(), path.1);
        }
        let shared = self.shared();
        let user = user_name(&self.app.slug);
        env.push(("HOME".into(), shared.to_string_lossy().into()));
        env.push(("USER".into(), user.clone()));
        env.push(("LOGNAME".into(), user));
        env.push(("LANG".into(), "C.UTF-8".into()));
        for (key, dir) in [
            ("npm_config_cache", "npm"),
            ("BUN_INSTALL_CACHE_DIR", "bun"),
            ("npm_config_store_dir", "pnpm"),
            ("YARN_CACHE_FOLDER", "yarn"),
            ("NUGET_PACKAGES", "nuget"),
        ] {
            env.push((
                key.into(),
                shared.join("cache").join(dir).to_string_lossy().into(),
            ));
        }
        let vars = env::all(&self.ctx.state, &self.app.id).await?;
        let managed = env::managed_for(&self.ctx.state, &self.app).await?;
        env.extend(env::pairs(&vars, &managed, &self.app.routes));
        Ok(dedup_last(env))
    }

    /// A Node app whose commands start with `bun` needs Bun on the path too, and the reverse.
    async fn extra_toolchain(&self) -> anyhow::Result<Option<PathBuf>> {
        let words = [&self.app.commands.install, &self.app.commands.build]
            .into_iter()
            .flatten()
            .filter_map(|c| c.split_whitespace().next());
        let mut wanted = None;
        for word in words {
            match word {
                "bun" | "bunx" => wanted = Some(RuntimeKind::Bun),
                "npm" | "npx" | "pnpm" | "yarn" | "node" | "corepack" => {
                    wanted = Some(RuntimeKind::Node)
                }
                _ => {}
            }
        }
        let Some(kind) = wanted.filter(|k| *k != self.app.toolchain) else {
            return Ok(None);
        };
        let mut installed: Vec<_> = toolchain::installed(&self.ctx.state)
            .await?
            .into_iter()
            .filter(|t| t.kind == kind)
            .collect();
        installed.sort_by_key(|t| version_key(&t.version));
        Ok(installed
            .last()
            .map(|t| bin_dir(&self.ctx.toolchains, kind, &t.version)))
    }

    fn shared(&self) -> PathBuf {
        app_dir(&self.app.slug).join("shared")
    }

    async fn say(&self, text: &str) -> anyhow::Result<()> {
        log::append(
            &self.ctx.state,
            &self.ctx.log,
            &self.deploy.id,
            SYSTEM,
            text,
        )
        .await?;
        Ok(())
    }

    async fn enter(&self, state: DeployState) -> anyhow::Result<()> {
        super::enter(&self.ctx.state, &self.deploy.id, state).await?;
        self.say(&format!("→ {}", state.as_str())).await
    }

    async fn skip(&self, state: DeployState, note: &str) -> anyhow::Result<()> {
        super::skip(&self.ctx.state, &self.deploy.id, state, note).await
    }
}

pub fn work_dir(release: &Path, root: &str) -> PathBuf {
    if root.trim().is_empty() {
        release.to_path_buf()
    } else {
        release.join(root.trim_matches('/'))
    }
}

pub fn looks_like_sha(git_ref: &str) -> bool {
    git_ref.len() >= 7 && git_ref.chars().all(|c| c.is_ascii_hexdigit())
}

fn bin_dir(store: &Store, kind: RuntimeKind, version: &str) -> PathBuf {
    let dir = store.dir(kind, version);
    let binary = runtime::by_kind(kind).binary();
    dir.join(binary)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(dir)
}

fn version_key(version: &str) -> Vec<u64> {
    version.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

fn dedup_last(env: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::with_capacity(env.len());
    for (key, value) in env {
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some(existing) => existing.1 = value,
            None => out.push((key, value)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_root_directory_nests_the_work_dir_and_a_sha_is_told_from_a_branch() {
        assert_eq!(work_dir(Path::new("/r"), ""), Path::new("/r"));
        assert_eq!(
            work_dir(Path::new("/r"), "apps/web/"),
            Path::new("/r/apps/web")
        );
        assert!(looks_like_sha("a3f9c2d4e81b06f5c9a2"));
        assert!(!looks_like_sha("main"));
        assert!(!looks_like_sha("release"));
        assert!(!looks_like_sha("abc"));
        assert!(looks_like_sha("deadbee"));
    }

    #[test]
    fn later_keys_win_and_versions_sort_numerically() {
        let env = dedup_last(vec![
            ("PATH".into(), "a".into()),
            ("X".into(), "1".into()),
            ("PATH".into(), "b".into()),
        ]);
        assert_eq!(
            env,
            vec![("PATH".into(), "b".into()), ("X".into(), "1".into())]
        );
        assert!(version_key("1.10.0") > version_key("1.9.3"));
    }
}
