pub mod commands;
pub mod domains;
pub mod env;
pub mod packages;
pub mod ports;
pub mod processes;
pub mod provision;
pub mod unit;
pub mod vhost;

use crate::detect;
use crate::manifest::Manifest;
use crate::runtime::{self, Commands, RuntimeKind};
use crate::state::State;
use crate::time;
use domains::{Domain, NewDomain};
use ferrum_platform::Platform;
use processes::{NewProcess, Process, WEB};
use serde::{Deserialize, Serialize};
use sqlx::Sqlite;

pub const SLUG_MAX: usize = 40;
const NAME_MAX: usize = 80;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("An application called {0} already exists.")]
    SlugTaken(String),
    #[error("No such application.")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error("A folder process has no program to run.")]
    NoProcess,
    #[error("The application has no domain {0}.")]
    DomainNotFound(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Route {
    pub path: String,
    pub process: String,
    pub websocket: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewRoute {
    pub path: String,
    pub process: String,
    #[serde(default)]
    pub websocket: bool,
}

impl NewRoute {
    pub fn main() -> Self {
        Self {
            path: "/".into(),
            process: WEB.into(),
            websocket: false,
        }
    }
}

impl From<&Route> for NewRoute {
    fn from(r: &Route) -> Self {
        Self {
            path: r.path.clone(),
            process: r.process.clone(),
            websocket: r.websocket,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct App {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub repository: String,
    /// The tag to deploy: the last one pushed, or the one picked at creation.
    pub git_ref: String,
    pub root: String,
    pub runtime: RuntimeKind,
    pub toolchain: RuntimeKind,
    pub runtime_version: String,
    pub commands: Commands,
    pub startup_budget_secs: u32,
    pub cpu_percent: u32,
    pub pause_for_migrations: bool,
    pub follow_repo_file: bool,
    pub processes: Vec<Process>,
    pub routes: Vec<Route>,
    pub packages: Vec<String>,
    pub domains: Vec<Domain>,
    pub current_release_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl App {
    pub fn process(&self, name: &str) -> Option<&Process> {
        self.processes.iter().find(|p| p.name == name)
    }

    pub fn command_processes(&self) -> impl Iterator<Item = &Process> {
        self.processes.iter().filter(|p| p.is_command())
    }

    pub fn port_processes(&self) -> impl Iterator<Item = &Process> {
        self.processes.iter().filter(|p| p.port.is_some())
    }

    pub fn port_of(&self, process: &str) -> Option<u16> {
        self.process(process).and_then(|p| p.port)
    }

    /// Each port process with its port, in order.
    pub fn ports(&self) -> Vec<(String, u16)> {
        self.processes
            .iter()
            .filter_map(|p| p.port.map(|port| (p.name.clone(), port)))
            .collect()
    }

    /// The port behind `/`: `web`'s when it has one, else the root route's process, else the
    /// first process with a port.
    pub fn main_port(&self) -> Option<u16> {
        self.port_of(WEB)
            .or_else(|| {
                self.routes
                    .iter()
                    .find(|r| r.path == "/")
                    .and_then(|r| self.port_of(&r.process))
            })
            .or_else(|| self.port_processes().next().and_then(|p| p.port))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NewApp {
    pub slug: String,
    pub name: String,
    pub repository: String,
    pub git_ref: String,
    pub root: String,
    pub runtime: RuntimeKind,
    pub toolchain: RuntimeKind,
    pub runtime_version: String,
    pub commands: Commands,
    pub startup_budget_secs: u32,
    pub cpu_percent: u32,
    pub pause_for_migrations: bool,
    pub follow_repo_file: bool,
    pub processes: Vec<NewProcess>,
    pub routes: Vec<NewRoute>,
    pub packages: Vec<String>,
    pub domains: Vec<NewDomain>,
    pub env: Vec<env::EnvVar>,
    pub env_hints: Vec<env::EnvHint>,
}

impl Default for NewApp {
    fn default() -> Self {
        Self {
            slug: String::new(),
            name: String::new(),
            repository: String::new(),
            git_ref: String::new(),
            root: String::new(),
            runtime: RuntimeKind::Node,
            toolchain: RuntimeKind::Node,
            runtime_version: String::new(),
            commands: Commands::default(),
            startup_budget_secs: 60,
            cpu_percent: 100,
            pause_for_migrations: true,
            follow_repo_file: false,
            processes: Vec::new(),
            routes: vec![NewRoute::main()],
            packages: Vec::new(),
            domains: Vec::new(),
            env: Vec::new(),
            env_hints: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppChanges {
    pub name: Option<String>,
    pub git_ref: Option<String>,
    pub root: Option<String>,
    pub runtime: Option<RuntimeKind>,
    pub toolchain: Option<RuntimeKind>,
    pub runtime_version: Option<String>,
    pub commands: Option<Commands>,
    pub startup_budget_secs: Option<u32>,
    pub cpu_percent: Option<u32>,
    pub pause_for_migrations: Option<bool>,
    pub follow_repo_file: Option<bool>,
    pub processes: Option<Vec<NewProcess>>,
    pub routes: Option<Vec<NewRoute>>,
    pub packages: Option<Vec<String>>,
    pub domains: Option<Vec<NewDomain>>,
}

pub fn valid_slug(slug: &str) -> bool {
    let bytes = slug.as_bytes();
    (1..=SLUG_MAX).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
}

fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains("..")
        && !path.contains("//")
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/-_.~".contains(c))
}

fn valid_repository(full_name: &str) -> bool {
    let Some((owner, repo)) = full_name.split_once('/') else {
        return false;
    };
    let ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    ok(owner) && ok(repo)
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::Invalid(message.into())
}

pub fn validate(new: &NewApp) -> Result<(), AppError> {
    if !valid_slug(&new.slug) {
        return Err(invalid(
            "A slug is 1 to 40 characters of lowercase letters, digits and hyphens, and cannot start or end with a hyphen.",
        ));
    }
    if new.name.trim().is_empty() || new.name.len() > NAME_MAX {
        return Err(invalid("An application needs a name."));
    }
    if !valid_repository(&new.repository) {
        return Err(invalid("The repository must be named owner/repo."));
    }
    if new.git_ref.trim().is_empty() || new.git_ref.contains("..") || new.git_ref.contains(' ') {
        return Err(invalid("Choose a tag to deploy."));
    }
    if new.root.starts_with('/') || new.root.contains("..") {
        return Err(invalid("The root directory is relative to the repository."));
    }
    if new.toolchain != new.runtime {
        return Err(invalid("The toolchain must match the runtime."));
    }
    if !runtime::by_kind(new.toolchain).valid_version(&new.runtime_version) {
        return Err(invalid(format!(
            "{} is not a full {} version.",
            new.runtime_version, new.toolchain
        )));
    }
    if new.processes.is_empty() {
        return Err(invalid("An application needs at least one process."));
    }
    for (i, process) in new.processes.iter().enumerate() {
        processes::validate(process).map_err(invalid)?;
        if new.processes[..i].iter().any(|p| p.name == process.name) {
            return Err(invalid(format!(
                "The process {} is listed twice.",
                process.name
            )));
        }
    }
    let build = new.commands.build.as_deref().unwrap_or("").trim();
    if new.processes.iter().any(NewProcess::is_folder) && build.is_empty() {
        return Err(invalid(
            "A folder process needs a build command that produces it.",
        ));
    }
    if new.routes.is_empty() {
        return Err(invalid("An application needs at least one route."));
    }
    for (i, route) in new.routes.iter().enumerate() {
        if !valid_path(&route.path) {
            return Err(invalid(format!(
                "{} is not a valid route path.",
                route.path
            )));
        }
        let Some(target) = new.processes.iter().find(|p| p.name == route.process) else {
            return Err(invalid(format!(
                "The route {} points at {}, which is not one of the processes.",
                route.path, route.process
            )));
        };
        if !target.is_folder() && !target.port {
            return Err(invalid(format!(
                "The route {} points at {}, which has no port to receive it.",
                route.path, route.process
            )));
        }
        if new.routes[..i].iter().any(|r| r.path == route.path) {
            return Err(invalid(format!(
                "The route {} is listed twice.",
                route.path
            )));
        }
    }
    for package in &new.packages {
        if !detect::valid_package(package) {
            return Err(invalid(format!("{package} is not a valid package name.")));
        }
    }
    domains::settle(&new.domains, &new.processes, &new.routes)?;
    for var in &new.env {
        env::valid_key(&var.key)?;
    }
    for hint in &new.env_hints {
        env::valid_key(&hint.key)?;
    }
    if !(10..=1600).contains(&new.cpu_percent) {
        return Err(invalid("CPU must be between 10% and 1600%."));
    }
    if !(5..=3600).contains(&new.startup_budget_secs) {
        return Err(invalid(
            "The startup budget must be between 5 and 3600 seconds.",
        ));
    }
    Ok(())
}

pub async fn create(state: &State, new: NewApp) -> anyhow::Result<App> {
    validate(&new)?;
    let settled = domains::settle(&new.domains, &new.processes, &new.routes)?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;

    let budget = new.startup_budget_secs as i64;
    let cpu = new.cpu_percent as i64;
    let inserted = sqlx::query!(
        "INSERT INTO apps (id, slug, name, repository, git_ref, tracking, root, runtime, toolchain,
                           runtime_version, install_cmd, build_cmd, migrate_cmd, startup_budget_secs,
                           cpu_percent, pause_for_migrations, follow_repo_file)
         VALUES (?, ?, ?, ?, ?, 'releases', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        id,
        new.slug,
        new.name,
        new.repository,
        new.git_ref,
        new.root,
        new.runtime,
        new.toolchain,
        new.runtime_version,
        new.commands.install,
        new.commands.build,
        new.commands.migrate,
        budget,
        cpu,
        new.pause_for_migrations,
        new.follow_repo_file,
    )
    .execute(&mut *tx)
    .await;
    if let Err(e) = inserted {
        if is_unique_violation(&e) {
            return Err(AppError::SlugTaken(new.slug).into());
        }
        return Err(e.into());
    }

    processes::write(&mut tx, &id, &new.processes).await?;
    write_routes(&mut tx, &id, &new.routes).await?;
    write_packages(&mut tx, &id, &new.packages).await?;
    domains::check_providers(&mut tx, &settled).await?;
    domains::write(&mut tx, &id, &settled).await?;
    for var in &new.env {
        env::set_in(&mut tx, &state.key, &id, &var.key, &var.value).await?;
    }
    env::replace_hints(&mut tx, &id, &new.env_hints).await?;
    tx.commit().await?;

    by_slug(state, &new.slug)
        .await?
        .ok_or_else(|| AppError::NotFound.into())
}

pub async fn update(state: &State, slug: &str, changes: AppChanges) -> anyhow::Result<App> {
    let current = by_slug(state, slug).await?.ok_or(AppError::NotFound)?;
    let processes: Vec<NewProcess> = match changes.processes {
        Some(processes) => processes,
        None => current.processes.iter().map(NewProcess::from).collect(),
    };
    let routes: Vec<NewRoute> = match changes.routes {
        Some(routes) => routes,
        None => current.routes.iter().map(NewRoute::from).collect(),
    };
    let merged = NewApp {
        slug: current.slug.clone(),
        name: changes.name.unwrap_or(current.name),
        repository: current.repository,
        git_ref: changes.git_ref.unwrap_or(current.git_ref),
        root: changes.root.unwrap_or(current.root),
        runtime: changes.runtime.unwrap_or(current.runtime),
        toolchain: changes.toolchain.unwrap_or(current.toolchain),
        runtime_version: changes.runtime_version.unwrap_or(current.runtime_version),
        commands: changes.commands.unwrap_or(current.commands),
        startup_budget_secs: changes
            .startup_budget_secs
            .unwrap_or(current.startup_budget_secs),
        cpu_percent: changes.cpu_percent.unwrap_or(current.cpu_percent),
        pause_for_migrations: changes
            .pause_for_migrations
            .unwrap_or(current.pause_for_migrations),
        follow_repo_file: changes.follow_repo_file.unwrap_or(current.follow_repo_file),
        processes,
        routes,
        packages: changes.packages.unwrap_or(current.packages),
        domains: changes
            .domains
            .unwrap_or_else(|| current.domains.iter().map(NewDomain::from).collect()),
        env: Vec::new(),
        env_hints: Vec::new(),
    };
    validate(&merged)?;
    let settled = domains::settle(&merged.domains, &merged.processes, &merged.routes)?;

    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let budget = merged.startup_budget_secs as i64;
    let cpu = merged.cpu_percent as i64;
    sqlx::query!(
        "UPDATE apps SET name = ?, git_ref = ?, root = ?, runtime = ?, toolchain = ?,
                         runtime_version = ?, install_cmd = ?, build_cmd = ?, migrate_cmd = ?,
                         startup_budget_secs = ?, cpu_percent = ?, pause_for_migrations = ?,
                         follow_repo_file = ?, updated_at = datetime('now')
         WHERE id = ?",
        merged.name,
        merged.git_ref,
        merged.root,
        merged.runtime,
        merged.toolchain,
        merged.runtime_version,
        merged.commands.install,
        merged.commands.build,
        merged.commands.migrate,
        budget,
        cpu,
        merged.pause_for_migrations,
        merged.follow_repo_file,
        current.id,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!("DELETE FROM app_routes WHERE app_id = ?", current.id)
        .execute(&mut *tx)
        .await?;
    processes::write(&mut tx, &current.id, &merged.processes).await?;
    write_routes(&mut tx, &current.id, &merged.routes).await?;
    let mut kept: Vec<&str> = merged
        .processes
        .iter()
        .filter(|p| p.port && !p.is_folder())
        .map(|p| p.name.as_str())
        .collect();
    kept.push(crate::redis::PORT_NAME);
    ports::release_unused(&mut tx, &current.id, &kept).await?;

    sqlx::query!("DELETE FROM app_packages WHERE app_id = ?", current.id)
        .execute(&mut *tx)
        .await?;
    write_packages(&mut tx, &current.id, &merged.packages).await?;
    domains::check_providers(&mut tx, &settled).await?;
    domains::write(&mut tx, &current.id, &settled).await?;
    tx.commit().await?;

    by_slug(state, slug)
        .await?
        .ok_or_else(|| AppError::NotFound.into())
}

/// The repo's file replaces the process list and paths when it states them, and each command
/// it states; everything it is silent on keeps its stored value. `[database]` speaks for the
/// first linked database only, so two databases never claim the same labels.
pub async fn apply_manifest(
    state: &State,
    platform: &dyn Platform,
    app: &App,
    manifest: &Manifest,
) -> anyhow::Result<App> {
    if let Some(spec) = &manifest.database
        && let Some(first) = crate::postgres::names_for(state, &app.id).await?.first()
    {
        crate::postgres::link_with_labels(state, platform, app, first, Some(spec)).await?;
    }
    if let Some(label) = manifest.redis.as_ref().and_then(|r| r.url.as_deref()) {
        crate::redis::set_label(state, &app.id, label).await?;
    }
    let mut commands = app.commands.clone();
    if manifest.commands.install.is_some() {
        commands.install = manifest.commands.install.clone();
    }
    if manifest.commands.build.is_some() {
        commands.build = manifest.commands.build.clone();
    }
    if manifest.commands.migrate.is_some() {
        commands.migrate = manifest.commands.migrate.clone();
    }
    let (processes, routes) = if manifest.states_processes() {
        (
            Some(manifest.processes.clone()),
            Some(manifest.routes.clone()),
        )
    } else {
        (None, None)
    };
    update(
        state,
        &app.slug,
        AppChanges {
            commands: Some(commands),
            processes,
            routes,
            ..AppChanges::default()
        },
    )
    .await
}

pub async fn set_git_ref(state: &State, id: &str, git_ref: &str) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE apps SET git_ref = ?, updated_at = datetime('now') WHERE id = ?",
        git_ref,
        id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn delete(state: &State, slug: &str) -> anyhow::Result<bool> {
    let done = sqlx::query!("DELETE FROM apps WHERE slug = ?", slug)
        .execute(&state.pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

async fn write_routes(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    app_id: &str,
    routes: &[NewRoute],
) -> anyhow::Result<()> {
    for route in routes {
        sqlx::query!(
            "INSERT INTO app_routes (app_id, path, port_name, process, websocket) VALUES (?, ?, ?, ?, ?)",
            app_id,
            route.path,
            route.process,
            route.process,
            route.websocket,
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn write_packages(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    app_id: &str,
    packages: &[String],
) -> anyhow::Result<()> {
    for name in packages {
        sqlx::query!(
            "INSERT OR IGNORE INTO app_packages (app_id, name) VALUES (?, ?)",
            app_id,
            name
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(db) if db.kind() == sqlx::error::ErrorKind::UniqueViolation)
}

pub async fn list(state: &State) -> anyhow::Result<Vec<App>> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", slug AS "slug!", name AS "name!", repository AS "repository!",
                  git_ref AS "git_ref!", root AS "root!",
                  runtime AS "runtime!: RuntimeKind", toolchain AS "toolchain!: RuntimeKind",
                  runtime_version AS "runtime_version!", install_cmd, build_cmd, migrate_cmd,
                  startup_budget_secs AS "startup_budget_secs!", cpu_percent AS "cpu_percent!",
                  pause_for_migrations AS "pause_for_migrations!: bool",
                  follow_repo_file AS "follow_repo_file!: bool", current_release_id,
                  created_at AS "created_at!", updated_at AS "updated_at!"
           FROM apps ORDER BY name, slug"#
    )
    .fetch_all(&state.pool)
    .await?;

    let mut apps = Vec::with_capacity(rows.len());
    for r in rows {
        let processes = processes::of(state, &r.id).await?;
        let routes = routes_of(state, &r.id).await?;
        let packages = packages_of(state, &r.id).await?;
        let domains = domains::of(state, &r.id).await?;
        apps.push(App {
            id: r.id,
            slug: r.slug,
            name: r.name,
            repository: r.repository,
            git_ref: r.git_ref,
            root: r.root,
            runtime: r.runtime,
            toolchain: r.toolchain,
            runtime_version: r.runtime_version,
            commands: Commands {
                install: r.install_cmd,
                build: r.build_cmd,
                migrate: r.migrate_cmd,
            },
            startup_budget_secs: r.startup_budget_secs as u32,
            cpu_percent: r.cpu_percent as u32,
            pause_for_migrations: r.pause_for_migrations,
            follow_repo_file: r.follow_repo_file,
            processes,
            routes,
            packages,
            domains,
            current_release_id: r.current_release_id,
            created_at: time::utc(r.created_at),
            updated_at: time::utc(r.updated_at),
        });
    }
    Ok(apps)
}

pub async fn by_slug(state: &State, slug: &str) -> anyhow::Result<Option<App>> {
    Ok(list(state).await?.into_iter().find(|a| a.slug == slug))
}

pub async fn by_id(state: &State, id: &str) -> anyhow::Result<Option<App>> {
    Ok(list(state).await?.into_iter().find(|a| a.id == id))
}

pub async fn by_repository(state: &State, repository: &str) -> anyhow::Result<Vec<App>> {
    Ok(list(state)
        .await?
        .into_iter()
        .filter(|a| a.repository == repository)
        .collect())
}

async fn routes_of(state: &State, app_id: &str) -> anyhow::Result<Vec<Route>> {
    let rows = sqlx::query!(
        r#"SELECT path AS "path!", process AS "process!", websocket AS "websocket!: bool"
           FROM app_routes WHERE app_id = ? ORDER BY length(path), path"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Route {
            path: r.path,
            process: r.process,
            websocket: r.websocket,
        })
        .collect())
}

async fn packages_of(state: &State, app_id: &str) -> anyhow::Result<Vec<String>> {
    let rows = sqlx::query!(
        r#"SELECT name AS "name!" FROM app_packages WHERE app_id = ? ORDER BY name"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.name).collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::apps::processes::ProcessKind;
    pub use crate::github::tests::state;
    use std::collections::HashSet;

    /// `(path, process, websocket)` rows; `main` is spelled `web`, and every other process
    /// named gets its own port and a start command of its own.
    pub fn new_app(slug: &str, routes: &[(&str, &str, bool)]) -> NewApp {
        let name = |n: &str| {
            if n == "main" {
                WEB.to_string()
            } else {
                n.to_string()
            }
        };
        let mut processes = vec![NewProcess::web("bun run start", None)];
        for (_, n, _) in routes {
            let n = name(n);
            if !processes.iter().any(|p| p.name == n) {
                processes.push(NewProcess {
                    name: n.clone(),
                    start: Some(format!("bun run {n}")),
                    ..NewProcess::default()
                });
            }
        }
        NewApp {
            slug: slug.into(),
            name: slug.into(),
            repository: "irixsoft/ledger".into(),
            git_ref: "main".into(),
            runtime_version: "22.11.0".into(),
            commands: Commands {
                install: Some("bun install --frozen-lockfile".into()),
                build: Some("bun run build".into()),
                migrate: None,
            },
            processes,
            routes: routes
                .iter()
                .map(|(path, n, ws)| NewRoute {
                    path: path.to_string(),
                    process: name(n),
                    websocket: *ws,
                })
                .collect(),
            domains: vec![format!("{slug}.example.com").as_str().into()],
            ..NewApp::default()
        }
    }

    pub fn rows(list: &[NewDomain]) -> Vec<Domain> {
        list.iter()
            .map(|d| Domain {
                domain: d.domain.clone(),
                job: d.job,
                target: d.target.clone(),
                primary: d.primary,
                wildcard: domains::is_wildcard(&d.domain),
                dns_provider_id: d.dns_provider_id.clone(),
            })
            .collect()
    }

    pub fn route(path: &str, process: &str, websocket: bool) -> Route {
        Route {
            path: path.into(),
            process: process.into(),
            websocket,
        }
    }

    pub fn process(name: &str, port: u16) -> Process {
        Process {
            name: name.into(),
            kind: ProcessKind::Command {
                start: "bun run start".into(),
            },
            dir: String::new(),
            port: Some(port),
            health_path: None,
            memory_mb: 512,
        }
    }

    pub fn worker(name: &str, start: &str) -> Process {
        Process {
            name: name.into(),
            kind: ProcessKind::Command {
                start: start.into(),
            },
            dir: String::new(),
            port: None,
            health_path: None,
            memory_mb: 512,
        }
    }

    pub fn folder(name: &str, static_dir: &str) -> Process {
        Process {
            name: name.into(),
            kind: ProcessKind::Folder {
                static_dir: static_dir.into(),
            },
            dir: String::new(),
            port: None,
            health_path: None,
            memory_mb: 512,
        }
    }

    pub fn app(slug: &str) -> App {
        App {
            id: "00000000-0000-0000-0000-000000000000".into(),
            slug: slug.into(),
            name: slug.into(),
            repository: "irixsoft/ledger".into(),
            git_ref: "main".into(),
            root: String::new(),
            runtime: RuntimeKind::Node,
            toolchain: RuntimeKind::Node,
            runtime_version: "22.11.0".into(),
            commands: Commands {
                install: Some("bun install --frozen-lockfile".into()),
                build: Some("bun run build".into()),
                migrate: None,
            },
            startup_budget_secs: 60,
            cpu_percent: 100,
            pause_for_migrations: true,
            follow_repo_file: false,
            processes: vec![process(WEB, 20000)],
            routes: vec![route("/", WEB, false)],
            packages: Vec::new(),
            domains: rows(&[NewDomain {
                primary: true,
                target: WEB.into(),
                ..format!("{slug}.example.com").as_str().into()
            }]),
            current_release_id: None,
            created_at: "2026-09-02T00:00:00Z".into(),
            updated_at: "2026-09-02T00:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn creating_an_app_allocates_one_port_per_port_process() {
        let (_d, state) = state().await;
        let app = create(
            &state,
            new_app("ledger", &[("/", "main", false), ("/ws", "ws", true)]),
        )
        .await
        .unwrap();
        let ports: HashSet<u16> = app.port_processes().filter_map(|p| p.port).collect();
        assert_eq!(ports.len(), 2);
        assert!(ports.iter().all(|p| ports::RANGE.contains(p)));
        assert_eq!(app.main_port(), app.port_of("web"));
        assert!(app.routes[1].websocket);
        assert_eq!(app.routes[1].process, "ws");
    }

    #[tokio::test]
    async fn two_routes_can_point_at_the_same_process() {
        let (_d, state) = state().await;
        let app = create(
            &state,
            new_app("ledger", &[("/", "main", false), ("/api", "main", false)]),
        )
        .await
        .unwrap();
        assert_eq!(app.processes.len(), 1);
        assert_eq!(
            app.port_of(&app.routes[0].process),
            app.port_of(&app.routes[1].process)
        );
    }

    #[tokio::test]
    async fn two_apps_never_share_a_port() {
        let (_d, state) = state().await;
        let a = create(&state, new_app("a", &[("/", "main", false)]))
            .await
            .unwrap();
        let b = create(&state, new_app("b", &[("/", "main", false)]))
            .await
            .unwrap();
        assert_ne!(a.main_port(), b.main_port());
    }

    #[tokio::test]
    async fn a_worker_and_a_folder_get_no_port() {
        let (_d, state) = state().await;
        let mut new = new_app("ledger", &[("/", "main", false)]);
        new.processes
            .push(NewProcess::worker("jobs", "bun run jobs"));
        new.processes
            .push(NewProcess::folder("admin", "apps/admin/dist"));
        let app = create(&state, new).await.unwrap();
        assert_eq!(app.processes.len(), 3);
        assert_eq!(app.port_of("jobs"), None);
        assert_eq!(app.port_of("admin"), None);
        assert!(app.process("admin").unwrap().static_dir().is_some());
        assert_eq!(app.command_processes().count(), 2);
        let ports: i64 = sqlx::query_scalar("SELECT count(*) FROM app_ports")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(ports, 1);
    }

    #[tokio::test]
    async fn deleting_an_app_frees_its_ports_its_processes_and_its_env() {
        let (_d, state) = state().await;
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        env::set(&state, &app.id, "SECRET", "x").await.unwrap();
        assert!(delete(&state, "ledger").await.unwrap());
        assert!(!delete(&state, "ledger").await.unwrap());
        let mut counts = Vec::new();
        for table in [
            "app_ports",
            "app_env",
            "app_domains",
            "app_env_hints",
            "app_processes",
        ] {
            let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&state.pool)
                .await
                .unwrap();
            counts.push(n);
        }
        assert_eq!(counts, [0, 0, 0, 0, 0]);
    }

    #[tokio::test]
    async fn env_hints_are_stored_at_creation_and_listed_behind_the_set_keys() {
        let (_d, state) = state().await;
        let mut wanted = new_app("ledger", &[("/", "main", false)]);
        wanted.env_hints = vec![
            env::EnvHint {
                key: "SMTP_HOST".into(),
                source: "from .env.example".into(),
                optional: true,
                suggest_app_url: false,
            },
            env::EnvHint {
                key: "STRIPE_KEY".into(),
                source: "from src/env.ts".into(),
                optional: false,
                suggest_app_url: false,
            },
        ];
        let app = create(&state, wanted).await.unwrap();
        env::set(&state, &app.id, "STRIPE_KEY", "sk").await.unwrap();
        env::add_hint(
            &state,
            &app.id,
            "STRIPE_KEY",
            "referenced in src/pay.ts",
            false,
        )
        .await
        .unwrap();
        env::add_hint(
            &state,
            &app.id,
            "MAIL_FROM",
            "referenced in src/mail.ts",
            false,
        )
        .await
        .unwrap();

        let entries = env::entries(&state, &app.id).await.unwrap();
        let shape: Vec<(&str, bool, Option<&str>, bool)> = entries
            .iter()
            .map(|e| (e.key.as_str(), e.set, e.source.as_deref(), e.optional))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("STRIPE_KEY", true, Some("from src/env.ts"), false),
                ("SMTP_HOST", false, Some("from .env.example"), true),
                ("MAIL_FROM", false, Some("referenced in src/mail.ts"), false),
            ],
            "a creation-time source is kept over a deploy-time one"
        );
    }

    #[tokio::test]
    async fn a_slug_must_be_a_valid_hostname_label_and_unit_name() {
        let (_d, state) = state().await;
        for bad in ["", "-a", "a-", "A", "a b", "a/b", "a..b", &"x".repeat(41)] {
            assert!(
                create(&state, new_app(bad, &[("/", "main", false)]))
                    .await
                    .is_err(),
                "{bad:?}"
            );
        }
        assert!(
            create(&state, new_app("my-app-2", &[("/", "main", false)]))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_duplicate_slug_is_a_conflict_not_a_500() {
        let (_d, state) = state().await;
        create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let mut second = new_app("ledger", &[("/", "main", false)]);
        second.domains = vec!["other.example.com".into()];
        let e = create(&state, second).await.unwrap_err();
        assert!(
            e.downcast_ref::<AppError>()
                .is_some_and(|e| matches!(e, AppError::SlugTaken(_)))
        );
    }

    #[tokio::test]
    async fn a_domain_belongs_to_one_app() {
        let (_d, state) = state().await;
        create(&state, new_app("a", &[("/", "main", false)]))
            .await
            .unwrap();
        let mut b = new_app("b", &[("/", "main", false)]);
        b.domains = vec!["a.example.com".into()];
        let e = create(&state, b).await.unwrap_err();
        assert!(e.to_string().contains("already belongs"), "{e}");
        assert!(
            by_slug(&state, "b").await.unwrap().is_none(),
            "nothing half-written"
        );
    }

    #[tokio::test]
    async fn validation_refuses_what_the_host_would_choke_on() {
        let (_d, state) = state().await;
        let mut bad_package = new_app("a", &[("/", "main", false)]);
        bad_package.packages = vec!["libvips; rm -rf /".into()];
        assert!(create(&state, bad_package).await.is_err());

        let mut bad_route = new_app("a", &[("api", "main", false)]);
        bad_route.slug = "a".repeat(3);
        assert!(create(&state, bad_route).await.is_err());

        let mut bad_version = new_app("a", &[("/", "main", false)]);
        bad_version.runtime_version = "22".into();
        assert!(create(&state, bad_version).await.is_err());

        let mut no_start = new_app("a", &[("/", "main", false)]);
        no_start.processes[0].start = None;
        assert!(create(&state, no_start).await.is_err());

        let mut no_processes = new_app("a", &[("/", "main", false)]);
        no_processes.processes.clear();
        assert!(create(&state, no_processes).await.is_err());

        let mut folder_without_build = new_app("a", &[("/", "main", false)]);
        folder_without_build.processes = vec![NewProcess::folder("web", "dist")];
        folder_without_build.commands.build = None;
        assert!(create(&state, folder_without_build).await.is_err());

        let mut route_to_nobody = new_app("a", &[("/", "main", false)]);
        route_to_nobody.routes.push(NewRoute {
            path: "/live".into(),
            process: "realtime".into(),
            websocket: true,
        });
        assert!(create(&state, route_to_nobody).await.is_err());

        let mut route_to_a_worker = new_app("a", &[("/", "main", false)]);
        route_to_a_worker
            .processes
            .push(NewProcess::worker("jobs", "bun run jobs"));
        route_to_a_worker.routes.push(NewRoute {
            path: "/jobs".into(),
            process: "jobs".into(),
            websocket: false,
        });
        assert!(create(&state, route_to_a_worker).await.is_err());

        let mut bad_domain = new_app("a", &[("/", "main", false)]);
        bad_domain.domains = vec!["203.0.113.9".into()];
        assert!(create(&state, bad_domain).await.is_err());
    }

    #[tokio::test]
    async fn updating_processes_keeps_ports_that_survive_and_frees_the_rest() {
        let (_d, state) = state().await;
        let app = create(
            &state,
            new_app("ledger", &[("/", "main", false), ("/ws", "ws", true)]),
        )
        .await
        .unwrap();
        let main_port = app.main_port().unwrap();

        let mut web = NewProcess::web("bun run start", Some("/healthz"));
        web.memory_mb = Some(1024);
        let updated = update(
            &state,
            "ledger",
            AppChanges {
                processes: Some(vec![
                    web,
                    NewProcess {
                        name: "metrics".into(),
                        start: Some("bun run metrics".into()),
                        ..NewProcess::default()
                    },
                ]),
                routes: Some(vec![
                    NewRoute::main(),
                    NewRoute {
                        path: "/metrics".into(),
                        process: "metrics".into(),
                        websocket: false,
                    },
                ]),
                ..AppChanges::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            updated.main_port(),
            Some(main_port),
            "the web port must not change"
        );
        let web = updated.process("web").unwrap();
        assert_eq!(web.memory_mb, 1024);
        assert_eq!(web.health_path.as_deref(), Some("/healthz"));
        assert!(updated.process("ws").is_none());
        assert!(updated.port_of("metrics").is_some());
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM app_ports")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(count, 2, "the ws port was released");
        assert!(updated.updated_at >= updated.created_at);
    }

    #[tokio::test]
    async fn a_manifest_replaces_the_processes_but_keeps_memory_limits_and_unstated_commands() {
        let (_d, state) = state().await;
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let mut web = NewProcess::web("bun run start", None);
        web.memory_mb = Some(2048);
        update(
            &state,
            "ledger",
            AppChanges {
                processes: Some(vec![web]),
                ..AppChanges::default()
            },
        )
        .await
        .unwrap();

        let toml = crate::manifest::parse_toml(
            "migrate = \"bun run db:migrate\"\n[processes.web]\nstart = \"bun run serve\"\ndir = \"apps/web\"\n[processes.realtime]\nstart = \"bun run rt\"\nport = true\npath = \"/live\"\nwebsocket = true\n[processes.jobs]\nstart = \"bun run jobs\"\n",
        )
        .unwrap();
        let manifest = crate::manifest::from_toml(&toml);
        let p = ferrum_platform::FakePlatform::new();
        let applied = apply_manifest(&state, &p, &app, &manifest).await.unwrap();

        let web = applied.process("web").unwrap();
        assert_eq!(web.start(), Some("bun run serve"));
        assert_eq!(web.dir, "apps/web");
        assert_eq!(web.memory_mb, 2048, "the limit belongs to the server");
        assert_eq!(applied.port_of("web"), app.port_of("web"));
        assert!(applied.port_of("realtime").is_some());
        assert_eq!(applied.port_of("jobs"), None);
        let live = applied.routes.iter().find(|r| r.path == "/live").unwrap();
        assert_eq!(live.process, "realtime");
        assert!(live.websocket);
        assert_eq!(
            applied.commands.migrate.as_deref(),
            Some("bun run db:migrate")
        );
        assert_eq!(
            applied.commands.build.as_deref(),
            Some("bun run build"),
            "a command the file does not state is kept"
        );

        let silent = crate::manifest::from_toml(
            &crate::manifest::parse_toml("packages = [\"ffmpeg\"]\n").unwrap(),
        );
        let kept = apply_manifest(&state, &p, &applied, &silent).await.unwrap();
        assert_eq!(
            kept.processes.len(),
            3,
            "a file silent on processes changes none"
        );
        assert!(p.sql().is_empty(), "no [database], no psql");
    }

    #[tokio::test]
    async fn a_database_section_labels_the_first_link_creates_its_roles_and_keeps_dropped_ones() {
        let (_d, state) = state().await;
        let p = ferrum_platform::FakePlatform::new();
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let db = crate::postgres::create(&state, &p, crate::postgres::tests::new("ledger_prod"))
            .await
            .unwrap();
        crate::postgres::create(&state, &p, crate::postgres::tests::new("analytics"))
            .await
            .unwrap();
        crate::postgres::roles::create(
            &state,
            &p,
            &db,
            crate::postgres::NewRole {
                name: "legacy".into(),
                ..crate::postgres::NewRole::default()
            },
        )
        .await
        .unwrap();
        crate::postgres::link(&state, &app.id, "ledger_prod")
            .await
            .unwrap();
        crate::postgres::link(&state, &app.id, "analytics")
            .await
            .unwrap();
        p.set_active("ferrum-redis-ledger");
        crate::redis::request(&state, &p, &app, 64).await.unwrap();

        let manifest = crate::manifest::from_toml(
            &crate::manifest::parse_toml(
                "[database]\nurl = \"DATABASE_ADMIN_URL\"\n[database.roles.app]\nurl = \"DATABASE_URL\"\n[redis]\nurl = \"CACHE_URL\"\n",
            )
            .unwrap(),
        );
        apply_manifest(&state, &p, &app, &manifest).await.unwrap();

        assert!(
            p.sql()
                .iter()
                .any(|s| s.contains("CREATE ROLE \"ledger_prod_app\""))
        );
        assert!(
            !p.sql().iter().any(|s| s.contains("analytics_app")),
            "only the first link"
        );
        assert_eq!(
            env::managed_for(&state, &app).await.unwrap().keys(),
            [
                "DATABASE_ADMIN_URL",
                "DATABASE_URL",
                "DATABASE_URL_LEGACY",
                "ANALYTICS_DATABASE_URL",
                "CACHE_URL"
            ]
        );
        let notices = crate::events::list(&state, 10, true).await.unwrap();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].kind, "role_kept");
        assert_eq!(notices[0].subject, "ledger_prod_legacy");
        assert!(
            notices[0].sentence.contains("ledger_prod_legacy"),
            "{}",
            notices[0].sentence
        );
    }
}
