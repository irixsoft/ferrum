pub mod archive;
pub mod detect;
pub mod exec;
pub mod fake;
pub mod ubuntu;

use std::path::{Path, PathBuf};
use std::time::Duration;

pub use detect::{
    Arch, HostInfo, OsRelease, Unsupported, check_supported, detect, parse_os_release,
};
pub use exec::{Exit, Stream};
pub use fake::FakePlatform;
pub use ubuntu::Ubuntu;

/// One command in a transient scope. `command` reaches `/bin/sh -c` as a single argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    pub unit: String,
    pub user: String,
    pub cwd: PathBuf,
    pub command: String,
    pub env: Vec<(String, String)>,
    pub memory_max_mb: u64,
    pub cpu_weight: u32,
    pub io_weight: u32,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalLine {
    pub at_usec: u64,
    pub priority: u8,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CgroupStats {
    pub memory_current: u64,
    pub memory_peak: u64,
    pub cpu_usage_usec: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ProcStat {
    pub busy_ticks: u64,
    pub total_ticks: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MemInfo {
    pub total_kb: u64,
    pub available_kb: u64,
    pub swap_total_kb: u64,
    pub swap_free_kb: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiskUsage {
    pub used_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FirewallRule {
    pub port: String,
    pub action: String,
    pub from: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Ban {
    pub ip: String,
    pub jail: String,
    pub banned_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
pub struct Sshd {
    pub port: u16,
    pub password_auth: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct KeyFingerprint {
    pub bits: u32,
    pub fingerprint: String,
    pub comment: String,
    pub kind: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("{cmd} exited with {code}: {stderr}")]
    Command {
        cmd: String,
        code: i32,
        stderr: String,
    },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Reload,
    ReloadOrRestart,
    Enable,
    EnableNow,
    Disable,
    Mask,
    DaemonReload,
}

impl ServiceAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::Reload => "reload",
            Self::ReloadOrRestart => "reload-or-restart",
            Self::Enable => "enable",
            Self::EnableNow => "enable-now",
            Self::Disable => "disable",
            Self::Mask => "mask",
            Self::DaemonReload => "daemon-reload",
        }
    }
}

pub trait Platform: Send + Sync {
    fn resolve_package(&self, name: &str) -> Vec<String>;
    fn install_packages(&self, names: &[&str]) -> Result<(), PlatformError>;
    fn add_apt_repo(&self, name: &str, key_url: &str, line: &str) -> Result<(), PlatformError>;
    fn service(&self, action: ServiceAction, unit: &str) -> Result<(), PlatformError>;
    fn service_is_active(&self, unit: &str) -> bool;
    fn write_file(&self, path: &Path, contents: &str, mode: u32) -> Result<(), PlatformError>;
    fn read_file(&self, path: &Path) -> Result<Option<String>, PlatformError>;
    fn file_exists(&self, path: &Path) -> bool;
    fn remove_file(&self, path: &Path) -> Result<(), PlatformError>;
    fn make_dirs(&self, path: &Path, mode: u32) -> Result<(), PlatformError>;
    fn remove_tree(&self, path: &Path) -> Result<(), PlatformError>;
    fn chown_tree(&self, path: &Path, user: &str) -> Result<(), PlatformError>;
    fn create_system_user(&self, name: &str, home: &Path) -> Result<(), PlatformError>;
    fn remove_system_user(&self, name: &str) -> Result<(), PlatformError>;
    fn extract_tar_gz(
        &self,
        archive: &[u8],
        dest: &Path,
        strip_components: u32,
    ) -> Result<(), PlatformError>;
    fn extract_zip(
        &self,
        archive: &[u8],
        dest: &Path,
        strip_components: u32,
    ) -> Result<(), PlatformError>;
    fn run_installer(
        &self,
        script: &Path,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<String, PlatformError>;
    fn cpu_has(&self, flag: &str) -> bool;
    fn postgres_sql(&self, database: &str, sql: &str) -> Result<String, PlatformError>;
    fn postgres_major_installed(&self) -> Option<u32>;
    fn postgres_dump(&self, database: &str, to: &Path) -> Result<(), PlatformError>;
    fn postgres_restore(&self, database: &str, from: &Path) -> Result<(), PlatformError>;
    /// A plain-SQL dump, which `pg_restore` cannot read, loaded through psql in one transaction.
    fn postgres_restore_sql(&self, database: &str, from: &Path) -> Result<(), PlatformError>;
    fn git_clone(
        &self,
        url: &str,
        git_ref: Option<&str>,
        dest: &Path,
        depth: u32,
    ) -> Result<(), PlatformError>;
    fn git_checkout(&self, dir: &Path, commit_sha: &str) -> Result<(), PlatformError>;
    fn git_head(&self, dir: &Path) -> Result<String, PlatformError>;
    fn git_scrub_remote(&self, dir: &Path, public_url: &str) -> Result<(), PlatformError>;
    fn run_scoped(
        &self,
        spec: &RunSpec,
        on_line: &mut dyn FnMut(Stream, &str),
    ) -> Result<Exit, PlatformError>;
    fn symlink_swap(&self, target: &Path, link: &Path) -> Result<(), PlatformError>;
    fn read_link(&self, link: &Path) -> Result<Option<PathBuf>, PlatformError>;
    fn list_dir(&self, dir: &Path) -> Result<Vec<String>, PlatformError>;
    fn disk_free_bytes(&self, path: &Path) -> Result<u64, PlatformError>;
    fn journal_tail(&self, unit: &str, lines: u32) -> Result<Vec<JournalLine>, PlatformError>;
    /// Blocks until `stopped` answers true; the unit's newest `lines` come first, then live ones.
    fn journal_follow(
        &self,
        unit: &str,
        lines: u32,
        on_line: &mut dyn FnMut(JournalLine),
        stopped: &dyn Fn() -> bool,
    ) -> Result<(), PlatformError>;
    fn cgroup_stats(&self, unit: &str) -> Result<Option<CgroupStats>, PlatformError>;
    fn proc_stat(&self) -> Result<ProcStat, PlatformError>;
    fn proc_meminfo(&self) -> Result<MemInfo, PlatformError>;
    fn uptime_secs(&self) -> Result<u64, PlatformError>;
    fn cpu_count(&self) -> usize;
    fn net_bytes(&self) -> Result<(u64, u64), PlatformError>;
    fn disk_usage(&self, path: &Path) -> Result<DiskUsage, PlatformError>;
    fn tail_file(&self, path: &Path, lines: u32) -> Result<Vec<String>, PlatformError>;
    fn nginx_test(&self) -> Result<(), PlatformError>;
    fn total_memory_kb(&self) -> Result<u64, PlatformError>;
    fn swap_total_kb(&self) -> Result<u64, PlatformError>;
    fn create_swapfile(&self, path: &Path, size_mb: u64) -> Result<(), PlatformError>;
    fn set_sysctl(&self, key: &str, value: &str) -> Result<(), PlatformError>;
    /// `None` while ufw is inactive or not installed.
    fn ufw_status(&self) -> Result<Option<Vec<FirewallRule>>, PlatformError>;
    /// Default deny in, allow out, then one `allow` per rule in the order given, then enable.
    fn ufw_apply(&self, allow: &[&str]) -> Result<(), PlatformError>;
    fn ufw_enable(&self) -> Result<(), PlatformError>;
    fn iptables_restore(&self, rules: &str) -> Result<(), PlatformError>;
    fn iptables_flush(&self) -> Result<(), PlatformError>;
    fn fail2ban_jails(&self) -> Result<Vec<String>, PlatformError>;
    fn fail2ban_bans(&self, jail: &str) -> Result<Vec<Ban>, PlatformError>;
    fn fail2ban_unban(&self, jail: &str, ip: &str) -> Result<(), PlatformError>;
    fn sshd_effective(&self) -> Result<Sshd, PlatformError>;
    fn sshd_test(&self) -> Result<(), PlatformError>;
    fn authorized_keys(&self) -> Result<Vec<KeyFingerprint>, PlatformError>;
    /// Runs `<binary> --self-check` and returns what it printed.
    fn self_check(&self, binary: &Path) -> Result<String, PlatformError>;
    /// Copies `from` over `to` atomically, keeping the previous file at `<to>.prev`.
    fn install_binary(&self, from: &Path, to: &Path) -> Result<(), PlatformError>;
    /// Schedules a restart of the unit through a transient timer, so the caller can be that unit.
    fn restart_later(&self, unit: &str) -> Result<(), PlatformError>;
}
