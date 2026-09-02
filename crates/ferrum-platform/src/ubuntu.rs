use crate::exec::{Exit, Spawn, Stream};
use crate::{Platform, PlatformError, RunSpec, ServiceAction, exec};
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
pub const PG_CONF_DIR: &str = "/etc/postgresql";
pub const PG_PORT: u16 = 5432;
pub const PGDG_KEY_URL: &str = "https://www.postgresql.org/media/keys/ACCC4CF8.asc";
pub const REDIS_SERVER: &str = "/usr/bin/redis-server";
pub const REDIS_DISTRO_UNIT: &str = "redis-server";
pub const REDIS_KEY_URL: &str = "https://packages.redis.io/gpg";
pub const GIT: &str = "/usr/bin/git";
pub const PG_USER: &str = "postgres";
const GIT_ENV: [(&str, &str); 1] = [("GIT_TERMINAL_PROMPT", "0")];
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

pub fn pg_conf_path(major: u32) -> PathBuf {
    Path::new(PG_CONF_DIR).join(format!("{major}/main/conf.d/ferrum.conf"))
}

/// Debian's `postgresql.service` is an umbrella that does not forward `reload`; the cluster
/// instance does.
pub fn pg_cluster_unit(major: u32) -> String {
    format!("postgresql@{major}-main")
}

pub fn pgdg_repo_line(codename: &str) -> String {
    format!("https://apt.postgresql.org/pub/repos/apt {codename}-pgdg main")
}

pub fn redis_repo_line(codename: &str) -> String {
    format!("https://packages.redis.io/deb {codename} main")
}

pub fn installed_pg_majors(
    entries: impl Iterator<Item = String>,
    has_main: impl Fn(u32) -> bool,
) -> Option<u32> {
    entries
        .filter_map(|name| name.parse::<u32>().ok())
        .filter(|major| has_main(*major))
        .max()
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

/// The scope accounts the build in its own cgroup, so `MemoryMax=` kills the build and nothing else.
pub fn scope_argv(spec: &RunSpec) -> Vec<String> {
    vec![
        "systemd-run".into(),
        "--scope".into(),
        "--quiet".into(),
        "--collect".into(),
        format!("--unit={}", spec.unit),
        format!("--uid={}", spec.user),
        format!("--gid={}", spec.user),
        format!("--working-directory={}", spec.cwd.display()),
        "-p".into(),
        format!("MemoryMax={}M", spec.memory_max_mb),
        "-p".into(),
        format!("CPUWeight={}", spec.cpu_weight),
        "-p".into(),
        format!("IOWeight={}", spec.io_weight),
        "--".into(),
        SH.into(),
        "-c".into(),
        spec.command.clone(),
    ]
}

pub fn scope_unit(unit: &str) -> String {
    format!("{unit}.scope")
}

/// `df --output=avail -B1 <path>` prints a header line and then the number.
pub fn parse_df_avail(output: &str) -> Option<u64> {
    output.lines().nth(1)?.trim().parse().ok()
}

pub fn redact_url(text: &str, url: &str) -> String {
    match url.split_once('@') {
        Some((credentials, _)) if credentials.contains("://") => {
            let scheme_end = credentials.find("://").map(|i| i + 3).unwrap_or(0);
            let public = format!(
                "{}{}",
                &credentials[..scheme_end],
                &url[credentials.len() + 1..]
            );
            text.replace(url, &public)
        }
        _ => text.to_string(),
    }
}

fn redacted(result: Result<String, PlatformError>, url: &str) -> Result<(), PlatformError> {
    match result {
        Ok(_) => Ok(()),
        Err(PlatformError::Command { cmd, code, stderr }) => Err(PlatformError::Command {
            cmd: redact_url(&cmd, url),
            code,
            stderr: redact_url(&stderr, url),
        }),
        Err(other) => Err(other),
    }
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

    fn read_file(&self, path: &Path) -> Result<Option<String>, PlatformError> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn file_exists(&self, path: &Path) -> bool {
        path.exists()
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

    fn postgres_sql(&self, database: &str, sql: &str) -> Result<String, PlatformError> {
        exec::run_with_stdin(
            &[
                "runuser",
                "-u",
                PG_USER,
                "--",
                "psql",
                "-X",
                "-q",
                "-A",
                "-t",
                "-v",
                "ON_ERROR_STOP=1",
                "-d",
                database,
            ],
            sql,
        )
    }

    fn postgres_major_installed(&self) -> Option<u32> {
        let entries = std::fs::read_dir(PG_CONF_DIR).ok()?;
        installed_pg_majors(
            entries
                .flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string)),
            |major| {
                Path::new(PG_CONF_DIR)
                    .join(format!("{major}/main"))
                    .is_dir()
            },
        )
    }

    fn postgres_dump(&self, database: &str, to: &Path) -> Result<(), PlatformError> {
        let to = to.to_string_lossy();
        exec::run(&[
            "runuser", "-u", PG_USER, "--", "pg_dump", "-Fc", "-f", &to, database,
        ])
        .map(|_| ())
    }

    fn postgres_restore(&self, database: &str, from: &Path) -> Result<(), PlatformError> {
        let from = from.to_string_lossy();
        exec::run(&[
            "runuser",
            "-u",
            PG_USER,
            "--",
            "pg_restore",
            "--single-transaction",
            "--exit-on-error",
            "-d",
            database,
            &from,
        ])
        .map(|_| ())
    }

    fn git_clone(
        &self,
        url: &str,
        git_ref: Option<&str>,
        dest: &Path,
        depth: u32,
    ) -> Result<(), PlatformError> {
        let depth = depth.to_string();
        let dest = dest.to_string_lossy();
        let mut argv = vec![
            "git",
            "clone",
            "--quiet",
            "--depth",
            &depth,
            "--single-branch",
        ];
        if let Some(git_ref) = git_ref {
            argv.extend(["--branch", git_ref]);
        }
        argv.extend([url, &dest]);
        redacted(exec::run_env(&argv, &GIT_ENV), url)
    }

    fn git_checkout(&self, dir: &Path, commit_sha: &str) -> Result<(), PlatformError> {
        let dir = dir.to_string_lossy();
        exec::run_env(
            &[
                "git", "-C", &dir, "fetch", "--quiet", "--depth", "1", "origin", commit_sha,
            ],
            &GIT_ENV,
        )?;
        exec::run_env(
            &[
                "git", "-C", &dir, "checkout", "--quiet", "--detach", commit_sha,
            ],
            &GIT_ENV,
        )
        .map(|_| ())
    }

    fn git_head(&self, dir: &Path) -> Result<String, PlatformError> {
        let dir = dir.to_string_lossy();
        exec::run_env(&["git", "-C", &dir, "rev-parse", "HEAD"], &GIT_ENV)
            .map(|out| out.trim().to_string())
    }

    fn git_scrub_remote(&self, dir: &Path, public_url: &str) -> Result<(), PlatformError> {
        let dir = dir.to_string_lossy();
        exec::run_env(
            &["git", "-C", &dir, "remote", "set-url", "origin", public_url],
            &GIT_ENV,
        )
        .map(|_| ())
    }

    fn run_scoped(
        &self,
        spec: &RunSpec,
        on_line: &mut dyn FnMut(Stream, &str),
    ) -> Result<Exit, PlatformError> {
        let argv = scope_argv(spec);
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let env: Vec<(&str, &str)> = spec
            .env
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let scope = scope_unit(&spec.unit);
        let kill = ["systemctl", "kill", "--signal=KILL", &scope];
        exec::run_streaming(
            &Spawn {
                argv: &argv,
                env: &env,
                clear_env: true,
                cwd: Some(&spec.cwd),
                timeout: Some(spec.timeout),
                on_timeout: Some(&kill),
            },
            on_line,
        )
    }

    fn symlink_swap(&self, target: &Path, link: &Path) -> Result<(), PlatformError> {
        let tmp = link.with_extension("tmp");
        ignore_missing(std::fs::remove_file(&tmp))?;
        std::os::unix::fs::symlink(target, &tmp)?;
        std::fs::rename(&tmp, link)?;
        Ok(())
    }

    fn read_link(&self, link: &Path) -> Result<Option<PathBuf>, PlatformError> {
        match std::fs::read_link(link) {
            Ok(target) => Ok(Some(target)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn list_dir(&self, dir: &Path) -> Result<Vec<String>, PlatformError> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .collect();
        names.sort();
        Ok(names)
    }

    fn disk_free_bytes(&self, path: &Path) -> Result<u64, PlatformError> {
        let out = exec::run(&["df", "--output=avail", "-B1", &path.to_string_lossy()])?;
        Ok(parse_df_avail(&out).unwrap_or(0))
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
    fn repository_lines_and_cluster_paths_follow_the_codename_and_the_major() {
        assert_eq!(
            pgdg_repo_line("noble"),
            "https://apt.postgresql.org/pub/repos/apt noble-pgdg main"
        );
        assert_eq!(
            redis_repo_line("jammy"),
            "https://packages.redis.io/deb jammy main"
        );
        assert_eq!(
            pg_conf_path(18),
            Path::new("/etc/postgresql/18/main/conf.d/ferrum.conf")
        );
        assert_eq!(pg_cluster_unit(18), "postgresql@18-main");
    }

    #[test]
    fn the_newest_cluster_with_a_main_instance_is_the_installed_major() {
        let entries = ["16", "18", "lost+found", "19"].map(String::from);
        assert_eq!(
            installed_pg_majors(entries.clone().into_iter(), |m| m != 19),
            Some(18)
        );
        assert_eq!(installed_pg_majors(entries.into_iter(), |_| false), None);
    }

    fn spec(command: &str) -> RunSpec {
        RunSpec {
            unit: "ferrum-build-ledger-1".into(),
            user: "ferrum-ledger".into(),
            cwd: PathBuf::from("/var/lib/ferrum/apps/ledger/releases/r1"),
            command: command.into(),
            env: vec![("PATH".into(), "/bin".into())],
            memory_max_mb: 1200,
            cpu_weight: 50,
            io_weight: 50,
            timeout: std::time::Duration::from_secs(60),
        }
    }

    #[test]
    fn the_scope_argv_carries_the_limits_and_never_the_command_as_more_than_one_element() {
        let argv = scope_argv(&spec("bun run build && echo 100%"));
        assert_eq!(argv[0], "systemd-run");
        assert!(argv.contains(&"--scope".to_string()));
        assert!(argv.contains(&"--uid=ferrum-ledger".to_string()));
        assert!(argv.contains(&"--unit=ferrum-build-ledger-1".to_string()));
        assert!(argv.windows(2).any(|w| w == ["-p", "MemoryMax=1200M"]));
        assert!(argv.windows(2).any(|w| w == ["-p", "CPUWeight=50"]));
        let sh = argv.iter().position(|a| a == "/bin/sh").unwrap();
        assert_eq!(argv[sh + 1], "-c");
        assert_eq!(argv[sh + 2], "bun run build && echo 100%");
        assert_eq!(argv.len(), sh + 3);
        assert!(
            !argv.iter().any(|a| a.contains("PATH")),
            "the environment travels on the process, not the command line"
        );
        assert_eq!(
            scope_unit("ferrum-build-ledger-1"),
            "ferrum-build-ledger-1.scope"
        );
    }

    #[test]
    fn df_output_is_read_past_its_header() {
        assert_eq!(
            parse_df_avail("      Avail\n41552420864\n"),
            Some(41_552_420_864)
        );
        assert_eq!(parse_df_avail(""), None);
    }

    #[test]
    fn credentials_are_stripped_from_an_error_that_echoes_the_url() {
        let url = "https://x-access-token:ghs_abc@github.com/irixsoft/ledger.git";
        let text = format!("git clone {url} failed for {url}");
        let clean = redact_url(&text, url);
        assert!(!clean.contains("ghs_abc"), "{clean}");
        assert!(clean.contains("https://github.com/irixsoft/ledger.git"));
        assert_eq!(redact_url("plain", "https://github.com/x.git"), "plain");
    }

    #[test]
    fn a_symlink_swap_is_a_rename_and_listings_are_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("current");
        std::fs::create_dir(dir.path().join("b")).unwrap();
        std::fs::create_dir(dir.path().join("a")).unwrap();
        Ubuntu.symlink_swap(&dir.path().join("a"), &link).unwrap();
        Ubuntu.symlink_swap(&dir.path().join("b"), &link).unwrap();
        assert_eq!(Ubuntu.read_link(&link).unwrap(), Some(dir.path().join("b")));
        assert_eq!(Ubuntu.read_link(&dir.path().join("nope")).unwrap(), None);
        assert!(!dir.path().join("current.tmp").exists());
        assert_eq!(
            Ubuntu.list_dir(dir.path()).unwrap(),
            vec!["a", "b", "current"]
        );
        assert!(
            Ubuntu
                .list_dir(&dir.path().join("nope"))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn sysctl_upsert_keeps_unrelated_keys() {
        let text = upsert_conf_line("vm.overcommit_memory = 1\n", "vm.swappiness", "10");
        assert!(text.contains("vm.overcommit_memory = 1"));
        assert!(text.contains("vm.swappiness = 10"));
    }
}
