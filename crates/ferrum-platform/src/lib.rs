pub mod detect;
pub mod ubuntu;

pub use detect::{Arch, HostInfo, Unsupported, check_supported, detect, parse_os_release};

pub trait Platform: Send + Sync {
    fn resolve_package(&self, name: &str) -> Vec<String>;
}
