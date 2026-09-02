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
    fn nginx_test(&self) -> Result<(), PlatformError>;
    fn total_memory_kb(&self) -> Result<u64, PlatformError>;
    fn swap_total_kb(&self) -> Result<u64, PlatformError>;
    fn create_swapfile(&self, path: &Path, size_mb: u64) -> Result<(), PlatformError>;
    fn set_sysctl(&self, key: &str, value: &str) -> Result<(), PlatformError>;
}
