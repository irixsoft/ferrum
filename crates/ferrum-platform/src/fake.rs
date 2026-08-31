use crate::{Platform, PlatformError, ServiceAction};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

#[derive(Default)]
struct Inner {
    calls: Vec<String>,
    files: HashMap<String, String>,
    fail_next: Option<String>,
    active: Vec<String>,
    memory_kb: u64,
    swap_kb: u64,
}

pub struct FakePlatform {
    inner: Mutex<Inner>,
}

impl Default for FakePlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl FakePlatform {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                memory_kb: 2_097_152,
                ..Inner::default()
            }),
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.inner.lock().unwrap().calls.clone()
    }

    pub fn fail_next(&self, contains: &str) {
        self.inner.lock().unwrap().fail_next = Some(contains.to_string());
    }

    pub fn set_memory_kb(&self, kb: u64) {
        self.inner.lock().unwrap().memory_kb = kb;
    }

    pub fn set_swap_kb(&self, kb: u64) {
        self.inner.lock().unwrap().swap_kb = kb;
    }

    pub fn set_active(&self, unit: &str) {
        self.inner.lock().unwrap().active.push(unit.to_string());
    }

    pub fn written(&self, path: &str) -> Option<String> {
        self.inner.lock().unwrap().files.get(path).cloned()
    }

    fn record(&self, call: String) -> Result<(), PlatformError> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(pat) = inner.fail_next.clone()
            && call.contains(&pat)
        {
            inner.fail_next = None;
            inner.calls.push(call.clone());
            return Err(PlatformError::Command {
                cmd: call,
                code: 1,
                stderr: "scripted failure".into(),
            });
        }
        inner.calls.push(call);
        Ok(())
    }
}

impl Platform for FakePlatform {
    fn resolve_package(&self, name: &str) -> Vec<String> {
        vec![name.to_string()]
    }

    fn install_packages(&self, names: &[&str]) -> Result<(), PlatformError> {
        self.record(format!("install_packages {}", names.join(" ")))
    }

    fn add_apt_repo(&self, name: &str, key_url: &str, repo: &str) -> Result<(), PlatformError> {
        self.record(format!("add_apt_repo {name} {key_url} {repo}"))
    }

    fn service(&self, action: ServiceAction, unit: &str) -> Result<(), PlatformError> {
        self.record(format!("service {} {unit}", action.as_str()))
    }

    fn service_is_active(&self, unit: &str) -> bool {
        self.inner.lock().unwrap().active.iter().any(|u| u == unit)
    }

    fn write_file(&self, path: &Path, contents: &str, mode: u32) -> Result<(), PlatformError> {
        let p = path.to_string_lossy().to_string();
        self.record(format!("write_file {p} {mode:o}"))?;
        self.inner
            .lock()
            .unwrap()
            .files
            .insert(p, contents.to_string());
        Ok(())
    }

    fn nginx_test(&self) -> Result<(), PlatformError> {
        self.record("nginx_test".to_string())
    }

    fn total_memory_kb(&self) -> Result<u64, PlatformError> {
        Ok(self.inner.lock().unwrap().memory_kb)
    }

    fn swap_total_kb(&self) -> Result<u64, PlatformError> {
        Ok(self.inner.lock().unwrap().swap_kb)
    }

    fn create_swapfile(&self, path: &Path, size_mb: u64) -> Result<(), PlatformError> {
        self.record(format!(
            "create_swapfile {} {size_mb}",
            path.to_string_lossy()
        ))
    }

    fn set_sysctl(&self, key: &str, value: &str) -> Result<(), PlatformError> {
        self.record(format!("set_sysctl {key} {value}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Platform, ServiceAction};
    use std::path::Path;

    #[test]
    fn fake_records_calls_in_order() {
        let p = FakePlatform::new();
        p.install_packages(&["nginx", "fail2ban"]).unwrap();
        p.service(ServiceAction::EnableNow, "nginx").unwrap();
        assert_eq!(
            p.calls(),
            vec![
                "install_packages nginx fail2ban".to_string(),
                "service enable-now nginx".to_string(),
            ]
        );
    }

    #[test]
    fn fake_can_be_scripted_to_fail() {
        let p = FakePlatform::new();
        p.fail_next("install_packages");
        assert!(p.install_packages(&["nginx"]).is_err());
        assert!(p.install_packages(&["nginx"]).is_ok());
    }

    #[test]
    fn fake_captures_written_files() {
        let p = FakePlatform::new();
        p.write_file(Path::new("/etc/nginx/x.conf"), "server {}", 0o644)
            .unwrap();
        assert_eq!(p.written("/etc/nginx/x.conf").as_deref(), Some("server {}"));
    }

    #[test]
    fn memory_and_swap_are_settable() {
        let p = FakePlatform::new();
        p.set_memory_kb(2_048_000);
        p.set_swap_kb(0);
        assert_eq!(p.total_memory_kb().unwrap(), 2_048_000);
        assert_eq!(p.swap_total_kb().unwrap(), 0);
    }
}
