pub mod env;
pub mod ports;
pub mod provision;
pub mod unit;
pub mod vhost;

use crate::detect;
use crate::dns::validate_hostname;
use crate::runtime::{self, Commands, Health, RuntimeKind};
use crate::state::State;
use crate::time;
use serde::{Deserialize, Serialize};
use sqlx::Sqlite;

pub const SLUG_MAX: usize = 40;
const NAME_MAX: usize = 80;
const PORT_NAME_MAX: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("An application called {0} already exists.")]
    SlugTaken(String),
    #[error("No such application.")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error("A static site has no process to run.")]
    NoProcess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum Tracking {
    Branch,
    Releases,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Route {
    pub path: String,
    pub port_name: String,
    pub port: u16,
    pub websocket: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NewRoute {
    pub path: String,
    pub port_name: String,
    #[serde(default)]
    pub websocket: bool,
}

impl NewRoute {
    pub fn main() -> Self {
        Self {
            path: "/".into(),
            port_name: "main".into(),
            websocket: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct App {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub repository: String,
    pub git_ref: String,
    pub tracking: Tracking,
    pub root: String,
    pub runtime: RuntimeKind,
    pub toolchain: RuntimeKind,
    pub runtime_version: String,
    pub commands: Commands,
    pub output_dir: Option<String>,
    pub health: Health,
    pub memory_mb: u32,
    pub cpu_percent: u32,
    pub pause_for_migrations: bool,
    pub routes: Vec<Route>,
    pub packages: Vec<String>,
    pub domains: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl App {
    pub fn main_port(&self) -> Option<u16> {
        self.routes
            .iter()
            .find(|r| r.path == "/")
            .or(self.routes.first())
            .map(|r| r.port)
    }

    pub fn primary_domain(&self) -> Option<&str> {
        self.domains.first().map(String::as_str)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct NewApp {
    pub slug: String,
    pub name: String,
    pub repository: String,
    pub git_ref: String,
    pub tracking: Tracking,
    pub root: String,
    pub runtime: RuntimeKind,
    pub toolchain: RuntimeKind,
    pub runtime_version: String,
    pub commands: Commands,
    pub output_dir: Option<String>,
    pub health: Health,
    pub memory_mb: u32,
    pub cpu_percent: u32,
    pub pause_for_migrations: bool,
    pub routes: Vec<NewRoute>,
    pub packages: Vec<String>,
    pub domains: Vec<String>,
    pub env: Vec<env::EnvVar>,
}

impl Default for NewApp {
    fn default() -> Self {
        Self {
            slug: String::new(),
            name: String::new(),
            repository: String::new(),
            git_ref: String::new(),
            tracking: Tracking::Releases,
            root: String::new(),
            runtime: RuntimeKind::Node,
            toolchain: RuntimeKind::Node,
            runtime_version: String::new(),
            commands: Commands::default(),
            output_dir: None,
            health: Health::default(),
            memory_mb: 512,
            cpu_percent: 100,
            pause_for_migrations: true,
            routes: vec![NewRoute::main()],
            packages: Vec::new(),
            domains: Vec::new(),
            env: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppChanges {
    pub name: Option<String>,
    pub git_ref: Option<String>,
    pub tracking: Option<Tracking>,
    pub root: Option<String>,
    pub runtime: Option<RuntimeKind>,
    pub toolchain: Option<RuntimeKind>,
    pub runtime_version: Option<String>,
    pub commands: Option<Commands>,
    pub output_dir: Option<String>,
    pub health: Option<Health>,
    pub memory_mb: Option<u32>,
    pub cpu_percent: Option<u32>,
    pub pause_for_migrations: Option<bool>,
    pub routes: Option<Vec<NewRoute>>,
    pub packages: Option<Vec<String>>,
    pub domains: Option<Vec<String>>,
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

fn valid_port_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=PORT_NAME_MAX).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
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
        return Err(invalid("Choose a branch or tag to deploy."));
    }
    if new.root.starts_with('/') || new.root.contains("..") {
        return Err(invalid("The root directory is relative to the repository."));
    }
    if !new.toolchain.installs_toolchain() {
        return Err(invalid(
            "A static site is built with Node, Bun or .NET; pick one.",
        ));
    }
    if new.runtime.has_process() && new.toolchain != new.runtime {
        return Err(invalid("The toolchain must match the runtime."));
    }
    if !runtime::by_kind(new.toolchain).valid_version(&new.runtime_version) {
        return Err(invalid(format!(
            "{} is not a full {} version.",
            new.runtime_version, new.toolchain
        )));
    }
    if new.runtime.has_process() {
        if new
            .commands
            .start
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            return Err(invalid("A server application needs a start command."));
        }
    } else {
        if new
            .commands
            .build
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            return Err(invalid("A static site needs a build command."));
        }
        match new.output_dir.as_deref().map(str::trim) {
            Some(dir) if !dir.is_empty() && !dir.starts_with('/') && !dir.contains("..") => {}
            _ => {
                return Err(invalid(
                    "A static site needs an output directory inside the build.",
                ));
            }
        }
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
        if !valid_port_name(&route.port_name) {
            return Err(invalid(format!(
                "{} is not a valid port name; use lowercase letters, digits and underscores.",
                route.port_name
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
    for (i, domain) in new.domains.iter().enumerate() {
        validate_hostname(domain).map_err(invalid)?;
        if new.domains[..i].iter().any(|d| d == domain) {
            return Err(invalid(format!("{domain} is listed twice.")));
        }
    }
    for var in &new.env {
        env::valid_key(&var.key)?;
    }
    if !(64..=65_536).contains(&new.memory_mb) {
        return Err(invalid("Memory must be between 64 MB and 64 GB."));
    }
    if !(10..=1600).contains(&new.cpu_percent) {
        return Err(invalid("CPU must be between 10% and 1600%."));
    }
    if !new.health.path.starts_with('/') {
        return Err(invalid("The health check path must start with /."));
    }
    if !(5..=3600).contains(&new.health.startup_budget_secs) {
        return Err(invalid(
            "The startup budget must be between 5 and 3600 seconds.",
        ));
    }
    Ok(())
}

pub async fn create(state: &State, new: NewApp) -> anyhow::Result<App> {
    validate(&new)?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;

    let health_budget = new.health.startup_budget_secs as i64;
    let memory = new.memory_mb as i64;
    let cpu = new.cpu_percent as i64;
    let inserted = sqlx::query!(
        "INSERT INTO apps (id, slug, name, repository, git_ref, tracking, root, runtime, toolchain,
                           runtime_version, install_cmd, build_cmd, start_cmd, migrate_cmd, output_dir,
                           health_path, startup_budget_secs, memory_mb, cpu_percent, pause_for_migrations)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        id,
        new.slug,
        new.name,
        new.repository,
        new.git_ref,
        new.tracking,
        new.root,
        new.runtime,
        new.toolchain,
        new.runtime_version,
        new.commands.install,
        new.commands.build,
        new.commands.start,
        new.commands.migrate,
        new.output_dir,
        new.health.path,
        health_budget,
        memory,
        cpu,
        new.pause_for_migrations,
    )
    .execute(&mut *tx)
    .await;
    if let Err(e) = inserted {
        if is_unique_violation(&e) {
            return Err(AppError::SlugTaken(new.slug).into());
        }
        return Err(e.into());
    }

    write_routes(&mut tx, &id, &new.routes).await?;
    write_packages(&mut tx, &id, &new.packages).await?;
    write_domains(&mut tx, &id, &new.domains).await?;
    for var in &new.env {
        env::set_in(&mut tx, &id, &var.key, &var.value).await?;
    }
    tx.commit().await?;

    by_slug(state, &new.slug)
        .await?
        .ok_or_else(|| AppError::NotFound.into())
}

pub async fn update(state: &State, slug: &str, changes: AppChanges) -> anyhow::Result<App> {
    let current = by_slug(state, slug).await?.ok_or(AppError::NotFound)?;
    let routes: Vec<NewRoute> = match &changes.routes {
        Some(routes) => routes.clone(),
        None => current
            .routes
            .iter()
            .map(|r| NewRoute {
                path: r.path.clone(),
                port_name: r.port_name.clone(),
                websocket: r.websocket,
            })
            .collect(),
    };
    let merged = NewApp {
        slug: current.slug.clone(),
        name: changes.name.unwrap_or(current.name),
        repository: current.repository,
        git_ref: changes.git_ref.unwrap_or(current.git_ref),
        tracking: changes.tracking.unwrap_or(current.tracking),
        root: changes.root.unwrap_or(current.root),
        runtime: changes.runtime.unwrap_or(current.runtime),
        toolchain: changes.toolchain.unwrap_or(current.toolchain),
        runtime_version: changes.runtime_version.unwrap_or(current.runtime_version),
        commands: changes.commands.unwrap_or(current.commands),
        output_dir: match changes.output_dir {
            Some(dir) if dir.trim().is_empty() => None,
            Some(dir) => Some(dir),
            None => current.output_dir,
        },
        health: changes.health.unwrap_or(current.health),
        memory_mb: changes.memory_mb.unwrap_or(current.memory_mb),
        cpu_percent: changes.cpu_percent.unwrap_or(current.cpu_percent),
        pause_for_migrations: changes
            .pause_for_migrations
            .unwrap_or(current.pause_for_migrations),
        routes,
        packages: changes.packages.unwrap_or(current.packages),
        domains: changes.domains.unwrap_or(current.domains),
        env: Vec::new(),
    };
    validate(&merged)?;

    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let health_budget = merged.health.startup_budget_secs as i64;
    let memory = merged.memory_mb as i64;
    let cpu = merged.cpu_percent as i64;
    sqlx::query!(
        "UPDATE apps SET name = ?, git_ref = ?, tracking = ?, root = ?, runtime = ?, toolchain = ?,
                         runtime_version = ?, install_cmd = ?, build_cmd = ?, start_cmd = ?,
                         migrate_cmd = ?, output_dir = ?, health_path = ?, startup_budget_secs = ?,
                         memory_mb = ?, cpu_percent = ?, pause_for_migrations = ?,
                         updated_at = datetime('now')
         WHERE id = ?",
        merged.name,
        merged.git_ref,
        merged.tracking,
        merged.root,
        merged.runtime,
        merged.toolchain,
        merged.runtime_version,
        merged.commands.install,
        merged.commands.build,
        merged.commands.start,
        merged.commands.migrate,
        merged.output_dir,
        merged.health.path,
        health_budget,
        memory,
        cpu,
        merged.pause_for_migrations,
        current.id,
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!("DELETE FROM app_routes WHERE app_id = ?", current.id)
        .execute(&mut *tx)
        .await?;
    write_routes(&mut tx, &current.id, &merged.routes).await?;
    let kept: Vec<&str> = merged.routes.iter().map(|r| r.port_name.as_str()).collect();
    ports::release_unused(&mut tx, &current.id, &kept).await?;

    sqlx::query!("DELETE FROM app_packages WHERE app_id = ?", current.id)
        .execute(&mut *tx)
        .await?;
    write_packages(&mut tx, &current.id, &merged.packages).await?;
    sqlx::query!("DELETE FROM app_domains WHERE app_id = ?", current.id)
        .execute(&mut *tx)
        .await?;
    write_domains(&mut tx, &current.id, &merged.domains).await?;
    tx.commit().await?;

    by_slug(state, slug)
        .await?
        .ok_or_else(|| AppError::NotFound.into())
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
        ports::allocate(tx, app_id, &route.port_name).await?;
        sqlx::query!(
            "INSERT INTO app_routes (app_id, path, port_name, websocket) VALUES (?, ?, ?, ?)",
            app_id,
            route.path,
            route.port_name,
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

async fn write_domains(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    app_id: &str,
    domains: &[String],
) -> anyhow::Result<()> {
    for (position, domain) in domains.iter().enumerate() {
        let position = position as i64;
        let inserted = sqlx::query!(
            "INSERT INTO app_domains (domain, app_id, position) VALUES (?, ?, ?)",
            domain,
            app_id,
            position
        )
        .execute(&mut **tx)
        .await;
        if let Err(e) = inserted {
            if is_unique_violation(&e) {
                return Err(
                    invalid(format!("{domain} already belongs to another application.")).into(),
                );
            }
            return Err(e.into());
        }
    }
    Ok(())
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(db) if db.kind() == sqlx::error::ErrorKind::UniqueViolation)
}

pub async fn list(state: &State) -> anyhow::Result<Vec<App>> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", slug AS "slug!", name AS "name!", repository AS "repository!",
                  git_ref AS "git_ref!", tracking AS "tracking!: Tracking", root AS "root!",
                  runtime AS "runtime!: RuntimeKind", toolchain AS "toolchain!: RuntimeKind",
                  runtime_version AS "runtime_version!", install_cmd, build_cmd, start_cmd, migrate_cmd,
                  output_dir, health_path AS "health_path!", startup_budget_secs AS "startup_budget_secs!",
                  memory_mb AS "memory_mb!", cpu_percent AS "cpu_percent!",
                  pause_for_migrations AS "pause_for_migrations!: bool", created_at AS "created_at!",
                  updated_at AS "updated_at!"
           FROM apps ORDER BY name, slug"#
    )
    .fetch_all(&state.pool)
    .await?;

    let mut apps = Vec::with_capacity(rows.len());
    for r in rows {
        let routes = routes_of(state, &r.id).await?;
        let packages = packages_of(state, &r.id).await?;
        let domains = domains_of(state, &r.id).await?;
        apps.push(App {
            id: r.id,
            slug: r.slug,
            name: r.name,
            repository: r.repository,
            git_ref: r.git_ref,
            tracking: r.tracking,
            root: r.root,
            runtime: r.runtime,
            toolchain: r.toolchain,
            runtime_version: r.runtime_version,
            commands: Commands {
                install: r.install_cmd,
                build: r.build_cmd,
                start: r.start_cmd,
                migrate: r.migrate_cmd,
            },
            output_dir: r.output_dir,
            health: Health {
                path: r.health_path,
                startup_budget_secs: r.startup_budget_secs as u32,
            },
            memory_mb: r.memory_mb as u32,
            cpu_percent: r.cpu_percent as u32,
            pause_for_migrations: r.pause_for_migrations,
            routes,
            packages,
            domains,
            created_at: time::utc(r.created_at),
            updated_at: time::utc(r.updated_at),
        });
    }
    Ok(apps)
}

pub async fn by_slug(state: &State, slug: &str) -> anyhow::Result<Option<App>> {
    Ok(list(state).await?.into_iter().find(|a| a.slug == slug))
}

async fn routes_of(state: &State, app_id: &str) -> anyhow::Result<Vec<Route>> {
    let rows = sqlx::query!(
        r#"SELECT r.path AS "path!", r.port_name AS "port_name!", r.websocket AS "websocket!: bool",
                  p.port AS "port!"
           FROM app_routes r JOIN app_ports p ON p.app_id = r.app_id AND p.name = r.port_name
           WHERE r.app_id = ? ORDER BY length(r.path), r.path"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Route {
            path: r.path,
            port_name: r.port_name,
            port: r.port as u16,
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

async fn domains_of(state: &State, app_id: &str) -> anyhow::Result<Vec<String>> {
    let rows = sqlx::query!(
        r#"SELECT domain AS "domain!" FROM app_domains WHERE app_id = ? ORDER BY position"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.domain).collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub use crate::github::tests::state;
    use std::collections::HashSet;

    pub fn new_app(slug: &str, routes: &[(&str, &str, bool)]) -> NewApp {
        NewApp {
            slug: slug.into(),
            name: slug.into(),
            repository: "irixsoft/ledger".into(),
            git_ref: "main".into(),
            tracking: Tracking::Branch,
            runtime_version: "22.11.0".into(),
            commands: Commands {
                install: Some("bun install --frozen-lockfile".into()),
                build: Some("bun run build".into()),
                start: Some("bun run start".into()),
                migrate: None,
            },
            routes: routes
                .iter()
                .map(|(path, name, ws)| NewRoute {
                    path: path.to_string(),
                    port_name: name.to_string(),
                    websocket: *ws,
                })
                .collect(),
            domains: vec![format!("{slug}.example.com")],
            ..NewApp::default()
        }
    }

    pub fn route(path: &str, name: &str, port: u16, websocket: bool) -> Route {
        Route {
            path: path.into(),
            port_name: name.into(),
            port,
            websocket,
        }
    }

    pub fn app(slug: &str) -> App {
        App {
            id: "00000000-0000-0000-0000-000000000000".into(),
            slug: slug.into(),
            name: slug.into(),
            repository: "irixsoft/ledger".into(),
            git_ref: "main".into(),
            tracking: Tracking::Branch,
            root: String::new(),
            runtime: RuntimeKind::Node,
            toolchain: RuntimeKind::Node,
            runtime_version: "22.11.0".into(),
            commands: Commands {
                install: Some("bun install --frozen-lockfile".into()),
                build: Some("bun run build".into()),
                start: Some("bun run start".into()),
                migrate: None,
            },
            output_dir: None,
            health: Health::default(),
            memory_mb: 512,
            cpu_percent: 100,
            pause_for_migrations: true,
            routes: vec![route("/", "main", 20000, false)],
            packages: Vec::new(),
            domains: vec![format!("{slug}.example.com")],
            created_at: "2026-09-02T00:00:00Z".into(),
            updated_at: "2026-09-02T00:00:00Z".into(),
        }
    }

    #[tokio::test]
    async fn creating_an_app_allocates_one_port_per_named_route() {
        let (_d, state) = state().await;
        let app = create(
            &state,
            new_app("ledger", &[("/", "main", false), ("/ws", "ws", true)]),
        )
        .await
        .unwrap();
        let ports: HashSet<u16> = app.routes.iter().map(|r| r.port).collect();
        assert_eq!(ports.len(), 2);
        assert!(ports.iter().all(|p| ports::RANGE.contains(p)));
        assert_eq!(app.main_port(), Some(app.routes[0].port));
        assert!(app.routes[1].websocket);
    }

    #[tokio::test]
    async fn two_routes_can_share_a_named_port() {
        let (_d, state) = state().await;
        let app = create(
            &state,
            new_app("ledger", &[("/", "main", false), ("/api", "main", false)]),
        )
        .await
        .unwrap();
        assert_eq!(app.routes[0].port, app.routes[1].port);
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
        assert_ne!(a.routes[0].port, b.routes[0].port);
    }

    #[tokio::test]
    async fn deleting_an_app_frees_its_ports_and_its_env() {
        let (_d, state) = state().await;
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        env::set(&state, &app.id, "SECRET", "x").await.unwrap();
        assert!(delete(&state, "ledger").await.unwrap());
        assert!(!delete(&state, "ledger").await.unwrap());
        let ports: i64 = sqlx::query_scalar("SELECT count(*) FROM app_ports")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let vars: i64 = sqlx::query_scalar("SELECT count(*) FROM app_env")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        let domains: i64 = sqlx::query_scalar("SELECT count(*) FROM app_domains")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!((ports, vars, domains), (0, 0, 0));
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
        no_start.commands.start = None;
        assert!(create(&state, no_start).await.is_err());

        let mut static_without_output = new_app("a", &[("/", "main", false)]);
        static_without_output.runtime = RuntimeKind::Static;
        assert!(create(&state, static_without_output).await.is_err());

        let mut bad_domain = new_app("a", &[("/", "main", false)]);
        bad_domain.domains = vec!["203.0.113.9".into()];
        assert!(create(&state, bad_domain).await.is_err());
    }

    #[tokio::test]
    async fn updating_routes_keeps_ports_that_survive_and_frees_the_rest() {
        let (_d, state) = state().await;
        let app = create(
            &state,
            new_app("ledger", &[("/", "main", false), ("/ws", "ws", true)]),
        )
        .await
        .unwrap();
        let main_port = app.main_port().unwrap();

        let updated = update(
            &state,
            "ledger",
            AppChanges {
                routes: Some(vec![
                    NewRoute::main(),
                    NewRoute {
                        path: "/metrics".into(),
                        port_name: "metrics".into(),
                        websocket: false,
                    },
                ]),
                memory_mb: Some(1024),
                ..AppChanges::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            updated.main_port(),
            Some(main_port),
            "the main port must not change"
        );
        assert_eq!(updated.memory_mb, 1024);
        assert!(updated.routes.iter().all(|r| r.port_name != "ws"));
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM app_ports")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(count, 2, "the ws port was released");
        assert!(updated.updated_at >= updated.created_at);
    }

    #[tokio::test]
    async fn an_update_that_fails_validation_changes_nothing() {
        let (_d, state) = state().await;
        create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let e = update(
            &state,
            "ledger",
            AppChanges {
                memory_mb: Some(1),
                ..AppChanges::default()
            },
        )
        .await
        .unwrap_err();
        assert!(e.downcast_ref::<AppError>().is_some());
        assert_eq!(
            by_slug(&state, "ledger").await.unwrap().unwrap().memory_mb,
            512
        );
    }

    #[tokio::test]
    async fn timestamps_are_unambiguous_utc() {
        let (_d, state) = state().await;
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        assert!(app.created_at.ends_with('Z'), "{}", app.created_at);
    }
}
