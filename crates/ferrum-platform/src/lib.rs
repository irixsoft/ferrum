pub mod detect;
pub mod exec;
pub mod fake;
pub mod ubuntu;

use std::path::Path;

pub use detect::{
    Arch, HostInfo, OsRelease, Unsupported, check_supported, detect, parse_os_release,
};
pub use fake::FakePlatform;
pub use ubuntu::Ubuntu;

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
    fn nginx_test(&self) -> Result<(), PlatformError>;
    fn total_memory_kb(&self) -> Result<u64, PlatformError>;
    fn swap_total_kb(&self) -> Result<u64, PlatformError>;
    fn create_swapfile(&self, path: &Path, size_mb: u64) -> Result<(), PlatformError>;
    fn set_sysctl(&self, key: &str, value: &str) -> Result<(), PlatformError>;
}
