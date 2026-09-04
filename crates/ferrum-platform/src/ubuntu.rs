use crate::exec::{Exit, Spawn, Stream};
use crate::{
    Ban, CgroupStats, DiskUsage, FirewallRule, JournalLine, KeyFingerprint, MemInfo, Platform,
    PlatformError, ProcStat, RunSpec, ServiceAction, Sshd, exec,
};
use std::io::{Read, Seek, SeekFrom};
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
pub const NGINX_LOG_DIR: &str = "/var/log/nginx";
pub const FAIL2BAN_UNIT: &str = "fail2ban";
pub const FAIL2BAN_JAIL_LOCAL: &str = "/etc/fail2ban/jail.d/ferrum.local";
pub const SSH_UNIT: &str = "ssh";
/// Sorts before cloud-init's `50-cloud-init.conf`; sshd keeps the first value it reads.
pub const SSHD_DROPIN: &str = "/etc/ssh/sshd_config.d/10-ferrum.conf";
pub const APT_AUTO_UPGRADES: &str = "/etc/apt/apt.conf.d/20auto-upgrades";
pub const ROOT_AUTHORIZED_KEYS: &str = "/root/.ssh/authorized_keys";
const HOME_DIR: &str = "/home";
const AUTHORIZED_KEYS: &str = ".ssh/authorized_keys";
const UFW_INACTIVE: &str = "Status: inactive";
const DEFAULT_SSH_PORT: u16 = 22;
const GIT_ENV: [(&str, &str); 1] = [("GIT_TERMINAL_PROMPT", "0")];
pub const FERRUM_BIN: &str = "/usr/local/bin/ferrum";
const SELF_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
const NOLOGIN: &str = "/usr/sbin/nologin";
const CPUINFO: &str = "/proc/cpuinfo";
const OS_RELEASE: &str = "/etc/os-release";
const CGROUP_SYSTEM_SLICE: &str = "/sys/fs/cgroup/system.slice";
const PROC_STAT: &str = "/proc/stat";
const PROC_MEMINFO: &str = "/proc/meminfo";
const PROC_UPTIME: &str = "/proc/uptime";
const PROC_NET_DEV: &str = "/proc/net/dev";
const JOURNAL_FIELDS: &str = "MESSAGE,PRIORITY,__REALTIME_TIMESTAMP";
const DEFAULT_PRIORITY: u8 = 6;
const TAIL_BLOCK: usize = 64 * 1024;

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

pub fn parse_df_used_size(output: &str) -> Option<DiskUsage> {
    let mut fields = output.lines().nth(1)?.split_whitespace();
    Some(DiskUsage {
        used_bytes: fields.next()?.parse().ok()?,
        total_bytes: fields.next()?.parse().ok()?,
    })
}

pub fn journal_argv(unit: &str, lines: &str, follow: bool) -> Vec<String> {
    let mut argv = vec![
        "journalctl".to_string(),
        "-u".into(),
        unit.into(),
        "-o".into(),
        "json".into(),
        "-n".into(),
        lines.into(),
        "-q".into(),
        "--no-pager".into(),
        format!("--output-fields={JOURNAL_FIELDS}"),
    ];
    if follow {
        argv.push("--follow".into());
    }
    argv
}

/// `MESSAGE` is a JSON string, or an array of bytes when the line was not UTF-8.
pub fn parse_journal_line(line: &str) -> Option<JournalLine> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let at_usec = value
        .get("__REALTIME_TIMESTAMP")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())?;
    let priority = value
        .get("PRIORITY")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PRIORITY);
    let message = match value.get("MESSAGE") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(bytes)) => {
            let raw: Vec<u8> = bytes
                .iter()
                .filter_map(|b| b.as_u64())
                .map(|b| b as u8)
                .collect();
            String::from_utf8_lossy(&raw).into_owned()
        }
        _ => String::new(),
    };
    Some(JournalLine {
        at_usec,
        priority,
        message,
    })
}

/// The first line of `/proc/stat`: `cpu user nice system idle iowait irq softirq steal …`.
pub fn parse_proc_stat(text: &str) -> Option<ProcStat> {
    let line = text.lines().find(|l| l.starts_with("cpu "))?;
    let ticks: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|f| f.parse().ok())
        .collect();
    if ticks.len() < 5 {
        return None;
    }
    let total: u64 = ticks.iter().sum();
    let idle = ticks[3] + ticks[4];
    Some(ProcStat {
        busy_ticks: total - idle,
        total_ticks: total,
    })
}

pub fn parse_cpu_stat_usec(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|l| l.strip_prefix("usage_usec "))
        .and_then(|v| v.trim().parse().ok())
}

pub fn parse_uptime_secs(text: &str) -> Option<u64> {
    let first = text.split_whitespace().next()?;
    Some(first.parse::<f64>().ok()? as u64)
}

/// Sums every interface but `lo`; `/proc/net/dev` puts received bytes first and sent bytes ninth.
pub fn parse_net_dev(text: &str) -> (u64, u64) {
    text.lines()
        .filter_map(|l| l.split_once(':'))
        .filter(|(name, _)| name.trim() != "lo")
        .fold((0, 0), |(rx, tx), (_, rest)| {
            let fields: Vec<u64> = rest
                .split_whitespace()
                .filter_map(|f| f.parse().ok())
                .collect();
            match (fields.first(), fields.get(8)) {
                (Some(r), Some(t)) => (rx + r, tx + t),
                _ => (rx, tx),
            }
        })
}

/// Reads backwards in blocks, so the tail of a large log costs the tail and not the file.
pub fn tail_lines<R: Read + Seek>(mut file: R, lines: usize) -> Result<Vec<String>, PlatformError> {
    if lines == 0 {
        return Ok(Vec::new());
    }
    let mut end = file.seek(SeekFrom::End(0))?;
    let mut collected: Vec<u8> = Vec::new();
    while end > 0 {
        let size = (end as usize).min(TAIL_BLOCK);
        end -= size as u64;
        file.seek(SeekFrom::Start(end))?;
        let mut block = vec![0u8; size];
        file.read_exact(&mut block)?;
        block.extend_from_slice(&collected);
        collected = block;
        let newlines = collected.iter().filter(|&&b| b == b'\n').count();
        let trailing = usize::from(collected.last() == Some(&b'\n'));
        if newlines - trailing >= lines {
            break;
        }
    }
    let text = String::from_utf8_lossy(&collected);
    let mut out: Vec<String> = text.lines().map(str::to_string).collect();
    if out.len() > lines {
        out.drain(..out.len() - lines);
    }
    Ok(out)
}

/// `ufw status numbered` rows: `[ 1] 22/tcp   ALLOW IN   Anywhere`; the `(v6)` twins are dropped.
pub fn parse_ufw_status(text: &str) -> Option<Vec<FirewallRule>> {
    if text.lines().any(|l| l.trim() == UFW_INACTIVE) {
        return None;
    }
    let rules = text
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix('[')?;
            let (_, rest) = rest.split_once(']')?;
            let cells: Vec<&str> = rest
                .split("  ")
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .collect();
            let [port, action, from, ..] = cells.as_slice() else {
                return None;
            };
            if port.ends_with("(v6)") {
                return None;
            }
            Some(FirewallRule {
                port: port.to_string(),
                action: action
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase(),
                from: from.to_string(),
            })
        })
        .collect();
    Some(rules)
}

pub fn parse_fail2ban_jails(text: &str) -> Vec<String> {
    text.lines()
        .find_map(|l| l.split_once("Jail list:"))
        .map(|(_, list)| {
            list.split(',')
                .map(str::trim)
                .filter(|j| !j.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `get <jail> banip --with-time` lines: `IP \tYYYY-MM-DD HH:MM:SS + secs = YYYY-MM-DD HH:MM:SS`.
pub fn parse_fail2ban_banip(text: &str, jail: &str) -> Vec<Ban> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let ip = fields.next()?;
            let banned_at = match (fields.next(), fields.next()) {
                (Some(date), Some(time)) if date.len() == 10 => Some(format!("{date}T{time}Z")),
                _ => None,
            };
            Some(Ban {
                ip: ip.to_string(),
                jail: jail.to_string(),
                banned_at,
            })
        })
        .collect()
}

/// `sshd -T` prints effective values in lowercase; the first `port` wins, as it does for sshd.
pub fn parse_sshd_t(text: &str) -> Sshd {
    let value = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(' '))
            .map(str::trim)
    };
    Sshd {
        port: value("port")
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_SSH_PORT),
        password_auth: value("passwordauthentication") != Some("no"),
    }
}

/// `ssh-keygen -lf` lines: `256 SHA256:… comment (ED25519)`.
pub fn parse_key_fingerprints(text: &str) -> Vec<KeyFingerprint> {
    text.lines()
        .filter_map(|line| {
            let (bits, rest) = line.trim().split_once(' ')?;
            let (fingerprint, rest) = rest.split_once(' ')?;
            let (comment, kind) = rest.rsplit_once(" (")?;
            Some(KeyFingerprint {
                bits: bits.parse().ok()?,
                fingerprint: fingerprint.to_string(),
                comment: comment.to_string(),
                kind: kind.trim_end_matches(')').to_string(),
            })
        })
        .collect()
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

fn absent_tool(result: Result<String, PlatformError>) -> Result<Option<String>, PlatformError> {
    match result {
        Ok(out) => Ok(Some(out)),
        Err(PlatformError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
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

    fn postgres_restore_sql(&self, database: &str, from: &Path) -> Result<(), PlatformError> {
        let from = from.to_string_lossy();
        exec::run(&[
            "runuser",
            "-u",
            PG_USER,
            "--",
            "psql",
            "-X",
            "-q",
            "-1",
            "-v",
            "ON_ERROR_STOP=1",
            "-d",
            database,
            "-f",
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
                stop: None,
            },
            on_line,
        )
    }

    fn journal_tail(&self, unit: &str, lines: u32) -> Result<Vec<JournalLine>, PlatformError> {
        let lines = lines.to_string();
        let argv = journal_argv(unit, &lines, false);
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        let out = exec::run(&argv)?;
        Ok(out.lines().filter_map(parse_journal_line).collect())
    }

    fn journal_follow(
        &self,
        unit: &str,
        lines: u32,
        on_line: &mut dyn FnMut(JournalLine),
        stopped: &dyn Fn() -> bool,
    ) -> Result<(), PlatformError> {
        let lines = lines.to_string();
        let argv = journal_argv(unit, &lines, true);
        let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
        exec::run_streaming(
            &Spawn {
                argv: &argv,
                env: &[],
                clear_env: false,
                cwd: None,
                timeout: None,
                on_timeout: None,
                stop: Some(stopped),
            },
            &mut |stream, line| {
                if stream == Stream::Stdout
                    && let Some(parsed) = parse_journal_line(line)
                {
                    on_line(parsed);
                }
            },
        )?;
        Ok(())
    }

    fn cgroup_stats(&self, unit: &str) -> Result<Option<CgroupStats>, PlatformError> {
        let dir = Path::new(CGROUP_SYSTEM_SLICE).join(format!("{unit}.service"));
        let current = match std::fs::read_to_string(dir.join("memory.current")) {
            Ok(text) => text.trim().parse().unwrap_or(0),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let peak = std::fs::read_to_string(dir.join("memory.peak"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(current);
        let cpu = std::fs::read_to_string(dir.join("cpu.stat"))
            .ok()
            .and_then(|text| parse_cpu_stat_usec(&text))
            .unwrap_or(0);
        Ok(Some(CgroupStats {
            memory_current: current,
            memory_peak: peak,
            cpu_usage_usec: cpu,
        }))
    }

    fn proc_stat(&self) -> Result<ProcStat, PlatformError> {
        let text = std::fs::read_to_string(PROC_STAT)?;
        Ok(parse_proc_stat(&text).unwrap_or(ProcStat {
            busy_ticks: 0,
            total_ticks: 0,
        }))
    }

    fn proc_meminfo(&self) -> Result<MemInfo, PlatformError> {
        let text = std::fs::read_to_string(PROC_MEMINFO)?;
        let field = |name| parse_meminfo_kb(&text, name).unwrap_or(0);
        Ok(MemInfo {
            total_kb: field("MemTotal"),
            available_kb: field("MemAvailable"),
            swap_total_kb: field("SwapTotal"),
            swap_free_kb: field("SwapFree"),
        })
    }

    fn uptime_secs(&self) -> Result<u64, PlatformError> {
        let text = std::fs::read_to_string(PROC_UPTIME)?;
        Ok(parse_uptime_secs(&text).unwrap_or(0))
    }

    fn cpu_count(&self) -> usize {
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1)
    }

    fn net_bytes(&self) -> Result<(u64, u64), PlatformError> {
        let text = std::fs::read_to_string(PROC_NET_DEV)?;
        Ok(parse_net_dev(&text))
    }

    fn disk_usage(&self, path: &Path) -> Result<DiskUsage, PlatformError> {
        let out = exec::run(&["df", "--output=used,size", "-B1", &path.to_string_lossy()])?;
        Ok(parse_df_used_size(&out).unwrap_or(DiskUsage {
            used_bytes: 0,
            total_bytes: 0,
        }))
    }

    fn tail_file(&self, path: &Path, lines: u32) -> Result<Vec<String>, PlatformError> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        tail_lines(file, lines as usize)
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

    fn ufw_status(&self) -> Result<Option<Vec<FirewallRule>>, PlatformError> {
        Ok(absent_tool(exec::run(&["ufw", "status", "numbered"]))?
            .and_then(|out| parse_ufw_status(&out)))
    }

    fn ufw_apply(&self, allow: &[&str], enable: bool) -> Result<(), PlatformError> {
        exec::run(&["ufw", "default", "deny", "incoming"])?;
        exec::run(&["ufw", "default", "allow", "outgoing"])?;
        for rule in allow {
            exec::run(&["ufw", "allow", rule])?;
        }
        if enable {
            exec::run(&["ufw", "--force", "enable"])?;
        }
        Ok(())
    }

    fn fail2ban_jails(&self) -> Result<Vec<String>, PlatformError> {
        let out = exec::run(&["fail2ban-client", "status"])?;
        Ok(parse_fail2ban_jails(&out))
    }

    fn fail2ban_bans(&self, jail: &str) -> Result<Vec<Ban>, PlatformError> {
        let out = exec::run(&["fail2ban-client", "get", jail, "banip", "--with-time"])?;
        Ok(parse_fail2ban_banip(&out, jail))
    }

    fn fail2ban_unban(&self, jail: &str, ip: &str) -> Result<(), PlatformError> {
        exec::run(&["fail2ban-client", "set", jail, "unbanip", ip]).map(|_| ())
    }

    fn sshd_effective(&self) -> Result<Sshd, PlatformError> {
        let out = exec::run(&["sshd", "-T"])?;
        Ok(parse_sshd_t(&out))
    }

    fn sshd_test(&self) -> Result<(), PlatformError> {
        exec::run(&["sshd", "-t"]).map(|_| ())
    }

    fn authorized_keys(&self) -> Result<Vec<KeyFingerprint>, PlatformError> {
        let mut files = vec![PathBuf::from(ROOT_AUTHORIZED_KEYS)];
        for home in self.list_dir(Path::new(HOME_DIR))? {
            files.push(Path::new(HOME_DIR).join(home).join(AUTHORIZED_KEYS));
        }
        let mut keys = Vec::new();
        for file in files.iter().filter(|f| f.is_file()) {
            if let Ok(out) = exec::run(&["ssh-keygen", "-lf", &file.to_string_lossy()]) {
                keys.extend(parse_key_fingerprints(&out));
            }
        }
        Ok(keys)
    }

    fn self_check(&self, binary: &Path) -> Result<String, PlatformError> {
        let binary = binary.to_string_lossy();
        let argv = [binary.as_ref(), "--self-check"];
        let mut out = String::new();
        let mut err = String::new();
        let exit = exec::run_streaming(
            &Spawn {
                argv: &argv,
                env: &[],
                clear_env: false,
                cwd: None,
                timeout: Some(SELF_CHECK_TIMEOUT),
                on_timeout: None,
                stop: None,
            },
            &mut |stream, line| {
                let sink = match stream {
                    Stream::Stdout => &mut out,
                    Stream::Stderr => &mut err,
                };
                sink.push_str(line);
                sink.push('\n');
            },
        )?;
        match exit {
            Exit::Code(0) => Ok(out.trim().to_string()),
            Exit::Code(code) => Err(PlatformError::Command {
                cmd: argv.join(" "),
                code,
                stderr: err.trim().to_string(),
            }),
            Exit::Killed { signal } => Err(PlatformError::Command {
                cmd: argv.join(" "),
                code: -1,
                stderr: format!("killed by signal {signal}"),
            }),
            Exit::TimedOut => Err(PlatformError::Command {
                cmd: argv.join(" "),
                code: -1,
                stderr: format!("no answer within {} seconds", SELF_CHECK_TIMEOUT.as_secs()),
            }),
        }
    }

    fn install_binary(&self, from: &Path, to: &Path) -> Result<(), PlatformError> {
        let new = to.with_extension("new");
        let prev = to.with_extension("prev");
        std::fs::copy(from, &new)?;
        std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755))?;
        ignore_missing(std::fs::rename(to, &prev))?;
        if let Err(e) = std::fs::rename(&new, to) {
            let _ = std::fs::rename(&prev, to);
            let _ = std::fs::remove_file(&new);
            return Err(e.into());
        }
        Ok(())
    }

    fn restart_later(&self, unit: &str) -> Result<(), PlatformError> {
        exec::run(&[
            "systemd-run",
            "--on-active=1s",
            &format!("--unit={unit}-restart"),
            "--collect",
            "systemctl",
            "restart",
            unit,
        ])
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable(dir: &Path, name: &str, script: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn a_self_check_returns_what_the_binary_printed_or_why_it_failed() {
        let dir = tempfile::tempdir().unwrap();
        let good = executable(
            dir.path(),
            "good",
            "#!/bin/sh\n[ \"$1\" = --self-check ] || exit 9\necho 'ferrum 9.9.9 (build x, commit y)'\n",
        );
        assert_eq!(
            Ubuntu.self_check(&good).unwrap(),
            "ferrum 9.9.9 (build x, commit y)"
        );

        let bad = executable(
            dir.path(),
            "bad",
            "#!/bin/sh\necho 'no such data dir' >&2\nexit 3\n",
        );
        match Ubuntu.self_check(&bad).unwrap_err() {
            PlatformError::Command { code, stderr, .. } => {
                assert_eq!(code, 3);
                assert_eq!(stderr, "no such data dir");
            }
            other => panic!("{other}"),
        }
    }

    #[test]
    fn installing_a_binary_keeps_the_previous_one_and_replaces_it_next_time() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("ferrum");
        std::fs::write(&to, b"first").unwrap();
        let staged = dir.path().join("update").join("ferrum");
        std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
        std::fs::write(&staged, b"second").unwrap();

        Ubuntu.install_binary(&staged, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"second");
        assert_eq!(std::fs::read(to.with_extension("prev")).unwrap(), b"first");
        assert_eq!(
            std::fs::metadata(&to).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(!to.with_extension("new").exists());
        assert!(staged.exists(), "the staged copy is the caller's to remove");

        std::fs::write(&staged, b"third").unwrap();
        Ubuntu.install_binary(&staged, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"third");
        assert_eq!(std::fs::read(to.with_extension("prev")).unwrap(), b"second");
    }

    #[test]
    fn a_first_install_has_no_previous_binary_to_keep() {
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("ferrum");
        let staged = dir.path().join("staged");
        std::fs::write(&staged, b"only").unwrap();
        Ubuntu.install_binary(&staged, &to).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"only");
        assert!(!to.with_extension("prev").exists());
    }

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

    const JOURNAL: &str = r#"{"PRIORITY":"6","__REALTIME_TIMESTAMP":"1788348289288518","__CURSOR":"s=34b3;i=1c3a8","_BOOT_ID":"ec6f","MESSAGE":"<info>  [1788348289.2882] device (wlp3s0): Activation: successful, device activated.","__SEQNUM":"115624"}"#;

    #[test]
    fn a_journal_line_is_read_with_its_priority_and_microsecond_stamp() {
        let line = parse_journal_line(JOURNAL).unwrap();
        assert_eq!(line.at_usec, 1_788_348_289_288_518);
        assert_eq!(line.priority, 6);
        assert!(line.message.starts_with("<info>  [1788348289.2882] device"));
    }

    #[test]
    fn a_non_utf8_message_arrives_lossy_and_a_missing_priority_is_info() {
        let raw = r#"{"__REALTIME_TIMESTAMP":"1788348290015095","MESSAGE":[104,105,255,33]}"#;
        let line = parse_journal_line(raw).unwrap();
        assert_eq!(line.message, "hi\u{fffd}!");
        assert_eq!(line.priority, 6);
        assert!(parse_journal_line("-- No entries --").is_none());
        assert!(parse_journal_line(r#"{"MESSAGE":"no stamp"}"#).is_none());
    }

    #[test]
    fn the_journal_argv_names_the_unit_the_fields_and_follows_on_request() {
        let argv = journal_argv("ferrum-app-ledger", "200", true);
        assert_eq!(argv[0], "journalctl");
        assert!(argv.windows(2).any(|w| w == ["-u", "ferrum-app-ledger"]));
        assert!(argv.windows(2).any(|w| w == ["-o", "json"]));
        assert!(argv.windows(2).any(|w| w == ["-n", "200"]));
        assert!(argv.contains(&"--no-pager".to_string()));
        assert!(
            argv.contains(&"--output-fields=MESSAGE,PRIORITY,__REALTIME_TIMESTAMP".to_string())
        );
        assert_eq!(argv.last().unwrap(), "--follow");
        assert!(!journal_argv("x", "10", false).contains(&"--follow".to_string()));
    }

    #[test]
    fn proc_stat_counts_busy_as_everything_but_idle_and_iowait() {
        let stat = parse_proc_stat(
            "cpu  1373644 872 299553 7623781 58382 81792 24374 0 0 0\ncpu0 1 2 3 4 5 6 7 0 0 0\n",
        )
        .unwrap();
        assert_eq!(stat.total_ticks, 9_462_398);
        assert_eq!(stat.busy_ticks, 9_462_398 - 7_623_781 - 58_382);
        assert!(parse_proc_stat("intr 1 2 3\n").is_none());
    }

    #[test]
    fn cgroup_uptime_and_net_readings_are_parsed_from_what_the_kernel_prints() {
        assert_eq!(
            parse_cpu_stat_usec("usage_usec 283900\nuser_usec 135230\nsystem_usec 148669\n"),
            Some(283_900)
        );
        assert_eq!(parse_uptime_secs("64914.25 76237.82\n"), Some(64_914));
        let dev = "Inter-|   Receive |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 2860571   30270    0    0    0     0          0         0  2860571   30270    0    0    0     0       0          0\nwlp3s0: 11560694   64357    0    0    0     0          0       109 73429128   79395    0    0    0     0       0          0\n";
        assert_eq!(parse_net_dev(dev), (11_560_694, 73_429_128));
        assert_eq!(
            parse_df_used_size("        Used    1B-blocks\n120622612480 248833376256\n"),
            Some(DiskUsage {
                used_bytes: 120_622_612_480,
                total_bytes: 248_833_376_256
            })
        );
    }

    #[test]
    fn a_tail_reads_only_the_end_of_a_large_file() {
        let mut text = String::new();
        for i in 0..50_000 {
            text.push_str(&format!("line {i} with some padding to make it longer\n"));
        }
        let file = std::io::Cursor::new(text.into_bytes());
        let tail = tail_lines(file, 3).unwrap();
        assert_eq!(
            tail,
            vec![
                "line 49997 with some padding to make it longer",
                "line 49998 with some padding to make it longer",
                "line 49999 with some padding to make it longer"
            ]
        );
        let short = tail_lines(std::io::Cursor::new(b"only\ntwo".to_vec()), 5).unwrap();
        assert_eq!(short, vec!["only", "two"]);
        assert!(
            tail_lines(std::io::Cursor::new(Vec::new()), 5)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn the_real_readers_answer_on_this_machine() {
        let stat = Ubuntu.proc_stat().unwrap();
        assert!(stat.total_ticks > stat.busy_ticks);
        let mem = Ubuntu.proc_meminfo().unwrap();
        assert!(mem.total_kb > mem.available_kb);
        assert!(Ubuntu.uptime_secs().unwrap() > 0);
        assert!(Ubuntu.cpu_count() >= 1);
        let disk = Ubuntu.disk_usage(Path::new("/")).unwrap();
        assert!(disk.total_bytes > disk.used_bytes);
        assert_eq!(
            Ubuntu.cgroup_stats("ferrum-app-no-such-unit").unwrap(),
            None
        );
        if exec::status(&["journalctl", "--version"]) {
            assert!(
                Ubuntu
                    .journal_tail("ferrum-app-no-such-unit", 5)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    const UFW_NUMBERED: &str = "Status: active\n\n     To                         Action      From\n     --                         ------      ----\n[ 1] 2222/tcp                   ALLOW IN    Anywhere\n[ 2] 80/tcp                     ALLOW IN    Anywhere\n[ 3] 443/tcp                    ALLOW IN    Anywhere\n[ 4] 2222/tcp (v6)              ALLOW IN    Anywhere (v6)\n[ 5] 80/tcp (v6)                ALLOW IN    Anywhere (v6)\n[ 6] 443/tcp (v6)               ALLOW IN    Anywhere (v6)\n\n";

    #[test]
    fn ufw_rules_are_read_once_each_and_an_inactive_firewall_is_none() {
        let rules = parse_ufw_status(UFW_NUMBERED).unwrap();
        assert_eq!(rules.len(), 3);
        assert_eq!(
            rules[0],
            FirewallRule {
                port: "2222/tcp".into(),
                action: "allow".into(),
                from: "Anywhere".into()
            }
        );
        assert_eq!(rules[2].port, "443/tcp");
        assert_eq!(parse_ufw_status("Status: inactive\n"), None);
        assert_eq!(parse_ufw_status("Status: active\n\n"), Some(Vec::new()));
    }

    const F2B_STATUS: &str =
        "Status\n|- Number of jail:\t2\n`- Jail list:\tnginx-botsearch, sshd\n";
    const F2B_BANIP: &str = "45.148.10.87 \t2026-09-02 10:12:33 + 3600 = 2026-09-02 11:12:33\n104.244.76.13 \t2026-09-02 10:40:01 + 3600 = 2026-09-02 11:40:01\n";

    #[test]
    fn fail2ban_output_yields_jails_and_timed_bans() {
        assert_eq!(
            parse_fail2ban_jails(F2B_STATUS),
            vec!["nginx-botsearch", "sshd"]
        );
        assert!(
            parse_fail2ban_jails("Status\n|- Number of jail:\t0\n`- Jail list:\t\n").is_empty()
        );
        let bans = parse_fail2ban_banip(F2B_BANIP, "sshd");
        assert_eq!(bans.len(), 2);
        assert_eq!(bans[0].ip, "45.148.10.87");
        assert_eq!(bans[0].jail, "sshd");
        assert_eq!(bans[0].banned_at.as_deref(), Some("2026-09-02T10:12:33Z"));
        assert!(parse_fail2ban_banip("\n", "sshd").is_empty());
        let bare = parse_fail2ban_banip("1.2.3.4\n", "sshd");
        assert_eq!(bare[0].banned_at, None);
    }

    #[test]
    fn sshd_t_gives_the_effective_port_and_password_setting() {
        let text = "port 2222\naddressfamily any\nlistenaddress [::]:2222\npasswordauthentication no\nkbdinteractiveauthentication no\n";
        assert_eq!(
            parse_sshd_t(text),
            Sshd {
                port: 2222,
                password_auth: false
            }
        );
        assert_eq!(
            parse_sshd_t("port 22\npasswordauthentication yes\n"),
            Sshd {
                port: 22,
                password_auth: true
            }
        );
        assert_eq!(parse_sshd_t("").port, 22);
        assert!(parse_sshd_t("").password_auth);
    }

    #[test]
    fn key_fingerprints_keep_a_spaced_comment_and_the_kind() {
        let text = "256 SHA256:puaulzlf91d/plw7qIdlrGgINNs66sk8c0vTfL1DAIs saeed@laptop (ED25519)\n2048 SHA256:W/u0nQg4RAkyIUjkJK1QdDUgARlzETA3K5m1OCIWuts no comment (RSA)\n";
        let keys = parse_key_fingerprints(text);
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].bits, 256);
        assert_eq!(keys[0].kind, "ED25519");
        assert_eq!(keys[0].comment, "saeed@laptop");
        assert!(keys[0].fingerprint.starts_with("SHA256:"));
        assert_eq!(keys[1].comment, "no comment");
        assert_eq!(keys[1].kind, "RSA");
        assert!(parse_key_fingerprints("nope: No such file or directory\n").is_empty());
    }

    #[test]
    fn the_key_and_sshd_tools_answer_on_this_machine() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("k");
        let made = exec::run(&[
            "ssh-keygen",
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-C",
            "ferrum test",
            "-f",
            &file.to_string_lossy(),
        ]);
        if made.is_err() {
            return;
        }
        let out = exec::run(&[
            "ssh-keygen",
            "-lf",
            &file.with_extension("pub").to_string_lossy(),
        ])
        .unwrap();
        let keys = parse_key_fingerprints(&out);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].comment, "ferrum test");
        assert_eq!(keys[0].kind, "ED25519");
        assert!(matches!(Ubuntu.ufw_status(), Ok(None) | Err(_)));
    }

    #[test]
    fn sysctl_upsert_keeps_unrelated_keys() {
        let text = upsert_conf_line("vm.overcommit_memory = 1\n", "vm.swappiness", "10");
        assert!(text.contains("vm.overcommit_memory = 1"));
        assert!(text.contains("vm.swappiness = 10"));
    }
}
