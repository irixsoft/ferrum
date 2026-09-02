use crate::{Platform, PlatformError, ServiceAction, exec};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub const NGINX_CONF_DIR: &str = "/etc/nginx/conf.d";
pub const NGINX_UNIT: &str = "nginx";
pub const KEYRING_DIR: &str = "/etc/apt/keyrings";
pub const SOURCES_DIR: &str = "/etc/apt/sources.list.d";
pub const SYSCTL_FILE: &str = "/etc/sysctl.d/99-ferrum.conf";
pub const FSTAB: &str = "/etc/fstab";
pub const SYSTEMD_UNIT_DIR: &str = "/etc/systemd/system";
pub const NGINX_CUSTOM_DIR: &str = "/etc/nginx/ferrum-custom";
pub const SYSTEM_PATH: &str = "/usr/local/bin:/usr/bin:/bin";
pub const SH: &str = "/bin/sh";
const NOLOGIN: &str = "/usr/sbin/nologin";
const CPUINFO: &str = "/proc/cpuinfo";
const OS_RELEASE: &str = "/etc/os-release";

const ICU_BY_CODENAME: [(&str, &str); 2] = [("jammy", "libicu70"), ("noble", "libicu74")];
const ICU_FALLBACK: &str = "libicu-dev";

const USERADD_EXISTS: i32 = 9;
const USERDEL_MISSING: i32 = 6;

const APT_ENV: [(&str, &str); 2] = [
    ("DEBIAN_FRONTEND", "noninteractive"),
    ("NEEDRESTART_MODE", "a"),
];

pub struct Ubuntu;

pub fn keyring_path(name: &str) -> PathBuf {
    Path::new(KEYRING_DIR).join(format!("{name}.asc"))
}

pub fn parse_meminfo_kb(text: &str, field: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix(field)?.strip_prefix(':'))
        .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
}

pub fn upsert_conf_line(existing: &str, key: &str, value: &str) -> String {
    let mut out: Vec<String> = existing
        .lines()
        .filter(|l| !l.split('=').next().unwrap_or("").trim().eq(key))
        .map(str::to_string)
        .collect();
    out.push(format!("{key} = {value}"));
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// Ubuntu names the ICU runtime by its major version, and .NET dies on start without it.
pub fn icu_package(codename: &str) -> &'static str {
    ICU_BY_CODENAME
        .iter()
        .find(|(name, _)| *name == codename)
        .map(|(_, pkg)| *pkg)
        .unwrap_or(ICU_FALLBACK)
}

pub fn cpu_flags_have(cpuinfo: &str, flag: &str) -> bool {
    cpuinfo
        .lines()
        .filter(|l| l.starts_with("flags"))
        .any(|l| l.split_whitespace().any(|f| f == flag))
}

fn tolerate(result: Result<String, PlatformError>, exit: i32) -> Result<(), PlatformError> {
    match result {
        Ok(_) => Ok(()),
        Err(PlatformError::Command { code, .. }) if code == exit => Ok(()),
        Err(e) => Err(e),
    }
}

fn ignore_missing(result: std::io::Result<()>) -> Result<(), PlatformError> {
    match result {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => Ok(other?),
    }
}

fn atomic_write(path: &Path, contents: &str, mode: u32) -> Result<(), PlatformError> {
    let dir = path.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.ferrum-tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("file")
    ));
    std::fs::write(&tmp, contents)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl Platform for Ubuntu {
    fn resolve_package(&self, name: &str) -> Vec<String> {
        if name == "libicu" {
            let codename = std::fs::read_to_string(OS_RELEASE)
                .ok()
                .and_then(|text| crate::parse_os_release(&text))
                .map(|os| os.codename)
                .unwrap_or_default();
            return vec![icu_package(&codename).to_string()];
        }
        vec![name.to_string()]
    }

    fn install_packages(&self, names: &[&str]) -> Result<(), PlatformError> {
        let mut argv = vec![
            "apt-get",
            "install",
            "-y",
            "-o",
            "DPkg::Lock::Timeout=120",
            "--no-install-recommends",
        ];
        argv.extend_from_slice(names);
        exec::run_env(&argv, &APT_ENV).map(|_| ())
    }

    fn add_apt_repo(&self, name: &str, key_url: &str, repo: &str) -> Result<(), PlatformError> {
        let keyring = keyring_path(name);
        std::fs::create_dir_all(KEYRING_DIR)?;
        exec::run(&["curl", "-fsSL", key_url, "-o", &keyring.to_string_lossy()])?;
        std::fs::set_permissions(&keyring, std::fs::Permissions::from_mode(0o644))?;

        let list = Path::new(SOURCES_DIR).join(format!("{name}.list"));
        let line = format!("deb [signed-by={}] {repo}\n", keyring.display());
        atomic_write(&list, &line, 0o644)?;

        exec::run_env(
            &["apt-get", "update", "-o", "DPkg::Lock::Timeout=120"],
            &APT_ENV,
        )
        .map(|_| ())
    }

    fn service(&self, action: ServiceAction, unit: &str) -> Result<(), PlatformError> {
        let argv = match action {
            ServiceAction::DaemonReload => vec!["systemctl", "daemon-reload"],
            ServiceAction::EnableNow => vec!["systemctl", "enable", "--now", unit],
            other => vec!["systemctl", other.as_str(), unit],
        };
        exec::run(&argv).map(|_| ())
    }

    fn service_is_active(&self, unit: &str) -> bool {
        exec::status(&["systemctl", "is-active", "--quiet", unit])
    }

    fn write_file(&self, path: &Path, contents: &str, mode: u32) -> Result<(), PlatformError> {
        atomic_write(path, contents, mode)
    }

    fn remove_file(&self, path: &Path) -> Result<(), PlatformError> {
        ignore_missing(std::fs::remove_file(path))
    }

    fn make_dirs(&self, path: &Path, mode: u32) -> Result<(), PlatformError> {
        std::fs::create_dir_all(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
        Ok(())
    }

    fn remove_tree(&self, path: &Path) -> Result<(), PlatformError> {
        ignore_missing(std::fs::remove_dir_all(path))
    }

    fn chown_tree(&self, path: &Path, user: &str) -> Result<(), PlatformError> {
        let owner = format!("{user}:{user}");
        exec::run(&["chown", "-R", &owner, &path.to_string_lossy()]).map(|_| ())
    }

    fn create_system_user(&self, name: &str, home: &Path) -> Result<(), PlatformError> {
        let home = home.to_string_lossy();
        tolerate(
            exec::run(&[
                "useradd",
                "--system",
                "--home-dir",
                &home,
                "--no-create-home",
                "--shell",
                NOLOGIN,
                "--user-group",
                name,
            ]),
            USERADD_EXISTS,
        )
    }

    fn remove_system_user(&self, name: &str) -> Result<(), PlatformError> {
        tolerate(exec::run(&["userdel", name]), USERDEL_MISSING)
    }

    fn extract_tar_gz(
        &self,
        archive: &[u8],
        dest: &Path,
        strip_components: u32,
    ) -> Result<(), PlatformError> {
        Ok(crate::archive::extract_tar_gz(
            archive,
            dest,
            strip_components,
        )?)
    }

    fn extract_zip(
        &self,
        archive: &[u8],
        dest: &Path,
        strip_components: u32,
    ) -> Result<(), PlatformError> {
        Ok(crate::archive::extract_zip(
            archive,
            dest,
            strip_components,
        )?)
    }

    fn run_installer(
        &self,
        script: &Path,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<String, PlatformError> {
        let script = script.to_string_lossy();
        let mut argv = vec!["bash", &script];
        argv.extend_from_slice(args);
        exec::run_env(&argv, env)
    }

    fn cpu_has(&self, flag: &str) -> bool {
        std::fs::read_to_string(CPUINFO)
            .map(|text| cpu_flags_have(&text, flag))
            .unwrap_or(false)
    }

    fn nginx_test(&self) -> Result<(), PlatformError> {
        exec::run(&["nginx", "-t"]).map(|_| ())
    }

    fn total_memory_kb(&self) -> Result<u64, PlatformError> {
        let text = std::fs::read_to_string("/proc/meminfo")?;
        Ok(parse_meminfo_kb(&text, "MemTotal").unwrap_or(0))
    }

    fn swap_total_kb(&self) -> Result<u64, PlatformError> {
        let text = std::fs::read_to_string("/proc/meminfo")?;
        Ok(parse_meminfo_kb(&text, "SwapTotal").unwrap_or(0))
    }

    fn create_swapfile(&self, path: &Path, size_mb: u64) -> Result<(), PlatformError> {
        let p = path.to_string_lossy().to_string();
        if exec::run(&["fallocate", "-l", &format!("{size_mb}M"), &p]).is_err() {
            exec::run(&[
                "dd",
                "if=/dev/zero",
                &format!("of={p}"),
                "bs=1M",
                &format!("count={size_mb}"),
            ])?;
        }
        exec::run(&["chmod", "600", &p])?;
        exec::run(&["mkswap", &p])?;
        exec::run(&["swapon", &p])?;

        let fstab = std::fs::read_to_string(FSTAB).unwrap_or_default();
        if !fstab
            .lines()
            .any(|l| l.split_whitespace().next() == Some(&p))
        {
            let mut updated = fstab;
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(&format!("{p} none swap sw 0 0\n"));
            atomic_write(Path::new(FSTAB), &updated, 0o644)?;
        }
        Ok(())
    }

    fn set_sysctl(&self, key: &str, value: &str) -> Result<(), PlatformError> {
        exec::run(&["sysctl", "-w", &format!("{key}={value}")])?;
        let existing = std::fs::read_to_string(SYSCTL_FILE).unwrap_or_default();
        atomic_write(
            Path::new(SYSCTL_FILE),
            &upsert_conf_line(&existing, key, value),
            0o644,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &str = "MemTotal:        2035440 kB\nMemFree:          123456 kB\nSwapTotal:             0 kB\nSwapFree:              0 kB\n";

    #[test]
    fn reads_memory_and_swap_totals() {
        assert_eq!(parse_meminfo_kb(MEMINFO, "MemTotal"), Some(2_035_440));
        assert_eq!(parse_meminfo_kb(MEMINFO, "SwapTotal"), Some(0));
        assert_eq!(parse_meminfo_kb(MEMINFO, "Nope"), None);
    }

    #[test]
    fn memfree_is_not_mistaken_for_memtotal() {
        assert_eq!(parse_meminfo_kb(MEMINFO, "MemFree"), Some(123_456));
    }

    #[test]
    fn sysctl_upsert_replaces_rather_than_appends() {
        let first = upsert_conf_line("", "vm.swappiness", "10");
        let second = upsert_conf_line(&first, "vm.swappiness", "20");
        assert_eq!(second, "vm.swappiness = 20\n");
    }

    #[test]
    fn icu_follows_the_release_and_falls_back_to_the_dev_package() {
        assert_eq!(icu_package("noble"), "libicu74");
        assert_eq!(icu_package("jammy"), "libicu70");
        assert_eq!(icu_package("plucky"), "libicu-dev");
    }

    #[test]
    fn cpu_flags_are_matched_whole() {
        let info = "processor\t: 0\nflags\t\t: fpu sse4_2 avx avx2 sha_ni\n";
        assert!(cpu_flags_have(info, "avx2"));
        assert!(!cpu_flags_have(info, "avx512f"));
        assert!(!cpu_flags_have(info, "av"));
    }

    #[test]
    fn an_existing_user_is_not_an_error_but_other_failures_are() {
        let exists = Err(PlatformError::Command {
            cmd: "useradd".into(),
            code: USERADD_EXISTS,
            stderr: "already exists".into(),
        });
        assert!(tolerate(exists, USERADD_EXISTS).is_ok());
        let syntax = Err(PlatformError::Command {
            cmd: "useradd".into(),
            code: 2,
            stderr: "invalid".into(),
        });
        assert!(tolerate(syntax, USERADD_EXISTS).is_err());
    }

    #[test]
    fn removing_what_is_already_gone_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        assert!(Ubuntu.remove_file(&dir.path().join("nope")).is_ok());
        assert!(Ubuntu.remove_tree(&dir.path().join("nope")).is_ok());
    }

    #[test]
    fn sysctl_upsert_keeps_unrelated_keys() {
        let text = upsert_conf_line("vm.overcommit_memory = 1\n", "vm.swappiness", "10");
        assert!(text.contains("vm.overcommit_memory = 1"));
        assert!(text.contains("vm.swappiness = 10"));
    }
}
