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
    cpu_flags: Vec<String>,
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

    pub fn removed(&self, path: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        !inner.files.contains_key(path)
            && inner
                .calls
                .iter()
                .any(|c| c == &format!("remove_file {path}") || c == &format!("remove_tree {path}"))
    }

    pub fn calls_matching(&self, prefix: &str) -> Vec<String> {
        self.calls()
            .into_iter()
            .filter(|c| c.starts_with(prefix))
            .collect()
    }

    pub fn set_cpu_flag(&self, flag: &str) {
        self.inner.lock().unwrap().cpu_flags.push(flag.to_string());
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

    fn remove_file(&self, path: &Path) -> Result<(), PlatformError> {
        let p = path.to_string_lossy().to_string();
        self.record(format!("remove_file {p}"))?;
        self.inner.lock().unwrap().files.remove(&p);
        Ok(())
    }

    fn make_dirs(&self, path: &Path, mode: u32) -> Result<(), PlatformError> {
        self.record(format!("make_dirs {} {mode:o}", path.to_string_lossy()))
    }

    fn remove_tree(&self, path: &Path) -> Result<(), PlatformError> {
        let p = path.to_string_lossy().to_string();
        self.record(format!("remove_tree {p}"))?;
        self.inner
            .lock()
            .unwrap()
            .files
            .retain(|k, _| !k.starts_with(&format!("{p}/")));
        Ok(())
    }

    fn chown_tree(&self, path: &Path, user: &str) -> Result<(), PlatformError> {
        self.record(format!("chown_tree {} {user}", path.to_string_lossy()))
    }

    fn create_system_user(&self, name: &str, home: &Path) -> Result<(), PlatformError> {
        self.record(format!(
            "create_system_user {name} {}",
            home.to_string_lossy()
        ))
    }

    fn remove_system_user(&self, name: &str) -> Result<(), PlatformError> {
        self.record(format!("remove_system_user {name}"))
    }

    fn extract_tar_gz(
        &self,
        archive: &[u8],
        dest: &Path,
        strip_components: u32,
    ) -> Result<(), PlatformError> {
        self.record(format!(
            "extract_tar_gz {} {strip_components}",
            dest.to_string_lossy()
        ))?;
        Ok(crate::archive::extract_tar_gz(
            archive,
            dest,
            strip_components,
        )?)
    }

    fn extract_zip(&self, archive: &[u8], dest: &Path) -> Result<(), PlatformError> {
        self.record(format!("extract_zip {}", dest.to_string_lossy()))?;
        Ok(crate::archive::extract_zip(archive, dest)?)
    }

    fn run_installer(
        &self,
        script: &Path,
        args: &[&str],
        _env: &[(&str, &str)],
    ) -> Result<String, PlatformError> {
        self.record(format!(
            "run_installer {} {}",
            script.to_string_lossy(),
            args.join(" ")
        ))?;
        Ok(String::new())
    }

    fn cpu_has(&self, flag: &str) -> bool {
        self.inner
            .lock()
            .unwrap()
            .cpu_flags
            .iter()
            .any(|f| f == flag)
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
    fn a_system_user_is_recorded_with_its_home_and_can_be_made_to_fail() {
        let p = FakePlatform::new();
        p.create_system_user("ferrum-ledger", Path::new("/var/lib/ferrum/apps/ledger"))
            .unwrap();
        assert_eq!(
            p.calls(),
            vec!["create_system_user ferrum-ledger /var/lib/ferrum/apps/ledger".to_string()]
        );
        p.fail_next("create_system_user");
        assert!(matches!(
            p.create_system_user("x", Path::new("/x")),
            Err(PlatformError::Command { .. })
        ));
    }

    #[test]
    fn removing_a_written_file_counts_as_removed() {
        let p = FakePlatform::new();
        p.write_file(Path::new("/etc/nginx/conf.d/a.conf"), "x", 0o644)
            .unwrap();
        assert!(!p.removed("/etc/nginx/conf.d/a.conf"));
        p.remove_file(Path::new("/etc/nginx/conf.d/a.conf"))
            .unwrap();
        assert!(p.removed("/etc/nginx/conf.d/a.conf"));
        assert!(!p.removed("/never/written"));
    }

    #[test]
    fn the_fake_really_extracts_so_callers_can_check_what_landed() {
        let dir = tempfile::tempdir().unwrap();
        let p = FakePlatform::new();
        let archive = crate::archive::tests::tar_gz(&[("top/bin/node", b"#!")]);
        p.extract_tar_gz(&archive, dir.path(), 1).unwrap();
        assert!(dir.path().join("bin/node").exists());
        assert_eq!(p.calls_matching("extract_tar_gz").len(), 1);
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
