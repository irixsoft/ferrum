use super::ports;
use crate::state::State;
use ferrum_platform::ubuntu::SYSTEMD_UNIT_DIR;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const NAME_MAX: usize = 16;
pub const DEFAULT_MEMORY_MB: u32 = 512;
pub const WEB: &str = "web";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ProcessKind {
    Command { start: String },
    Folder { static_dir: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Process {
    pub name: String,
    #[serde(flatten)]
    pub kind: ProcessKind,
    pub dir: String,
    pub port: Option<u16>,
    pub health_path: Option<String>,
    pub memory_mb: u32,
}

impl Process {
    pub fn start(&self) -> Option<&str> {
        match &self.kind {
            ProcessKind::Command { start } => Some(start),
            ProcessKind::Folder { .. } => None,
        }
    }

    pub fn static_dir(&self) -> Option<&str> {
        match &self.kind {
            ProcessKind::Folder { static_dir } => Some(static_dir),
            ProcessKind::Command { .. } => None,
        }
    }

    pub fn is_command(&self) -> bool {
        matches!(self.kind, ProcessKind::Command { .. })
    }

    pub fn unit_name(&self, slug: &str) -> String {
        unit_name(slug, &self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NewProcess {
    pub name: String,
    pub start: Option<String>,
    pub dir: String,
    pub port: bool,
    pub health: Option<String>,
    pub static_dir: Option<String>,
    pub memory_mb: Option<u32>,
}

impl Default for NewProcess {
    fn default() -> Self {
        Self {
            name: String::new(),
            start: None,
            dir: String::new(),
            port: true,
            health: None,
            static_dir: None,
            memory_mb: None,
        }
    }
}

impl NewProcess {
    pub fn web(start: &str, health: Option<&str>) -> Self {
        Self {
            name: WEB.into(),
            start: Some(start.into()),
            health: health.map(str::to_string),
            ..Self::default()
        }
    }

    pub fn worker(name: &str, start: &str) -> Self {
        Self {
            name: name.into(),
            start: Some(start.into()),
            port: false,
            ..Self::default()
        }
    }

    pub fn folder(name: &str, static_dir: &str) -> Self {
        Self {
            name: name.into(),
            static_dir: Some(static_dir.into()),
            port: false,
            ..Self::default()
        }
    }

    pub fn is_folder(&self) -> bool {
        self.static_dir.is_some()
    }
}

impl From<&Process> for NewProcess {
    fn from(p: &Process) -> Self {
        Self {
            name: p.name.clone(),
            start: p.start().map(str::to_string),
            dir: p.dir.clone(),
            port: p.port.is_some(),
            health: p.health_path.clone(),
            static_dir: p.static_dir().map(str::to_string),
            memory_mb: Some(p.memory_mb),
        }
    }
}

pub fn unit_name(slug: &str, process: &str) -> String {
    format!("ferrum-app-{slug}-{process}")
}

pub fn unit_path(slug: &str, process: &str) -> PathBuf {
    Path::new(SYSTEMD_UNIT_DIR).join(format!("{}.service", unit_name(slug, process)))
}

pub fn unit_prefix(slug: &str) -> String {
    format!("ferrum-app-{slug}-")
}

pub fn legacy_unit_name(slug: &str) -> String {
    format!("ferrum-app-{slug}")
}

pub fn legacy_unit_path(slug: &str) -> PathBuf {
    Path::new(SYSTEMD_UNIT_DIR).join(format!("{}.service", legacy_unit_name(slug)))
}

pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    name != crate::redis::PORT_NAME
        && (1..=NAME_MAX).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

pub fn valid_dir(dir: &str) -> bool {
    !dir.starts_with('/')
        && !dir.split('/').any(|part| part == "..")
        && !dir
            .chars()
            .any(|c| c.is_whitespace() || c == '\'' || c == '\\')
}

pub fn validate(p: &NewProcess) -> Result<(), String> {
    if !valid_name(&p.name) {
        return Err(format!(
            "{} is not a valid process name; use up to {NAME_MAX} lowercase letters, digits and underscores, and not redis.",
            p.name
        ));
    }
    let start = p.start.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let folder = p
        .static_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match (start, folder) {
        (None, None) => {
            return Err(format!(
                "The process {} needs a start command or a folder to serve.",
                p.name
            ));
        }
        (Some(_), Some(_)) => {
            return Err(format!(
                "The process {} is either a command or a folder, not both.",
                p.name
            ));
        }
        (None, Some(dir)) => {
            if !valid_dir(dir) {
                return Err(format!(
                    "The folder for {} is relative to the application, without ..",
                    p.name
                ));
            }
            if p.port || p.health.is_some() {
                return Err(format!(
                    "The folder process {} has no port and no health check.",
                    p.name
                ));
            }
        }
        (Some(_), None) => {}
    }
    if !valid_dir(&p.dir) {
        return Err(format!(
            "The start folder for {} is relative to the application, without ..",
            p.name
        ));
    }
    if let Some(health) = &p.health {
        if !p.port {
            return Err(format!(
                "The process {} has no port, so it cannot have a health check path.",
                p.name
            ));
        }
        if !health.starts_with('/') {
            return Err(format!(
                "The health check path for {} must start with /.",
                p.name
            ));
        }
    }
    if let Some(memory) = p.memory_mb
        && !(64..=65_536).contains(&memory)
    {
        return Err(format!(
            "Memory for {} must be between 64 MB and 64 GB.",
            p.name
        ));
    }
    Ok(())
}

pub async fn of(state: &State, app_id: &str) -> anyhow::Result<Vec<Process>> {
    let rows = sqlx::query!(
        r#"SELECT p.name AS "name!", p.start, p.dir AS "dir!", p.has_port AS "has_port!: bool",
                  p.health_path, p.static_dir, p.memory_mb AS "memory_mb!", o.port AS "port?: i64"
           FROM app_processes p
           LEFT JOIN app_ports o ON o.app_id = p.app_id AND o.name = p.name
           WHERE p.app_id = ? ORDER BY p.position, p.name"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Process {
            name: r.name,
            kind: match r.static_dir {
                Some(static_dir) => ProcessKind::Folder { static_dir },
                None => ProcessKind::Command {
                    start: r.start.unwrap_or_default(),
                },
            },
            dir: r.dir,
            port: if r.has_port {
                r.port.map(|p| p as u16)
            } else {
                None
            },
            health_path: r.health_path,
            memory_mb: r.memory_mb as u32,
        })
        .collect())
}

/// Replaces the app's processes. A memory limit not given keeps the one already stored for
/// that name, since the limit belongs to the server and never comes from the repo.
pub(super) async fn write(
    tx: &mut Transaction<'_, Sqlite>,
    app_id: &str,
    processes: &[NewProcess],
    taken_on_host: &(dyn Fn(u16) -> bool + Send + Sync),
) -> anyhow::Result<()> {
    let existing: HashMap<String, i64> = sqlx::query!(
        r#"SELECT name AS "name!", memory_mb AS "memory_mb!" FROM app_processes WHERE app_id = ?"#,
        app_id
    )
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .map(|r| (r.name, r.memory_mb))
    .collect();
    sqlx::query!("DELETE FROM app_processes WHERE app_id = ?", app_id)
        .execute(&mut **tx)
        .await?;
    for (position, p) in processes.iter().enumerate() {
        let folder = p.is_folder();
        let start = if folder { None } else { p.start.clone() };
        let has_port = p.port && !folder;
        let memory = p
            .memory_mb
            .map(i64::from)
            .or_else(|| existing.get(&p.name).copied())
            .unwrap_or(DEFAULT_MEMORY_MB as i64);
        let position = position as i64;
        sqlx::query!(
            "INSERT INTO app_processes (app_id, name, start, dir, has_port, health_path, static_dir, memory_mb, position)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            app_id,
            p.name,
            start,
            p.dir,
            has_port,
            p.health,
            p.static_dir,
            memory,
            position,
        )
        .execute(&mut **tx)
        .await?;
        if has_port {
            ports::allocate(tx, app_id, &p.name, taken_on_host).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_is_a_command_or_a_folder_never_both_or_neither() {
        let neither = NewProcess {
            name: "web".into(),
            ..NewProcess::default()
        };
        assert!(validate(&neither).is_err());
        let both = NewProcess {
            name: "web".into(),
            start: Some("bun run start".into()),
            static_dir: Some("dist".into()),
            ..NewProcess::default()
        };
        assert!(validate(&both).is_err());
        assert!(validate(&NewProcess::web("bun run start", Some("/healthz"))).is_ok());
        assert!(validate(&NewProcess::folder("admin", "apps/admin/dist")).is_ok());
    }

    #[test]
    fn a_folder_has_no_port_and_no_health_check_and_a_worker_has_no_health_check() {
        let mut folder = NewProcess::folder("admin", "dist");
        folder.port = true;
        assert!(validate(&folder).is_err());
        let mut worker = NewProcess::worker("jobs", "bun run start");
        assert!(validate(&worker).is_ok());
        worker.health = Some("/healthz".into());
        assert!(validate(&worker).is_err());
    }

    #[test]
    fn names_and_folders_are_kept_safe() {
        for name in ["web", "realtime", "jobs_2", "a"] {
            assert!(valid_name(name), "{name}");
        }
        for name in ["", "Web", "redis", "2fast", "a-b", "toolongtoolongtoolong"] {
            assert!(!valid_name(name), "{name}");
        }
        for dir in ["", "apps/web", ".medusa/server"] {
            assert!(valid_dir(dir), "{dir}");
        }
        for dir in ["/etc", "../x", "apps/../..", "a b", "it's"] {
            assert!(!valid_dir(dir), "{dir}");
        }
    }

    #[test]
    fn unit_names_carry_the_slug_and_the_process() {
        assert_eq!(unit_name("ledger", "web"), "ferrum-app-ledger-web");
        assert_eq!(
            unit_path("ledger", "jobs"),
            Path::new("/etc/systemd/system/ferrum-app-ledger-jobs.service")
        );
        assert_eq!(legacy_unit_name("ledger"), "ferrum-app-ledger");
        assert_eq!(unit_prefix("ledger"), "ferrum-app-ledger-");
    }
}
