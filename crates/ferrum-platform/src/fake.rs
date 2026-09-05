use crate::exec::{Exit, Stream};
use crate::{
    Ban, CgroupStats, DiskUsage, FirewallRule, JournalLine, KeyFingerprint, MemInfo, Platform,
    PlatformError, ProcStat, RunSpec, ServiceAction, Sshd,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const FAKE_HEAD: &str = "0000000000000000000000000000000000000000";
const FOLLOW_POLL: Duration = Duration::from_millis(10);

type Latch = Arc<(Mutex<bool>, Condvar)>;

#[derive(Default)]
struct Inner {
    calls: Vec<String>,
    files: HashMap<String, String>,
    dirs: HashSet<String>,
    links: HashMap<String, String>,
    fail_next: Option<String>,
    active: Vec<String>,
    cpu_flags: Vec<String>,
    memory_kb: u64,
    swap_kb: u64,
    disk_free: u64,
    sql: Vec<String>,
    answers: Vec<(String, String)>,
    postgres_major: Option<u32>,
    head: String,
    scripts: Vec<(String, Vec<String>, Exit)>,
    runs: Vec<RunSpec>,
    gates: Vec<(String, Latch)>,
    journal: HashMap<String, Vec<JournalLine>>,
    cgroups: HashMap<String, CgroupStats>,
    proc_stat: ProcStat,
    meminfo: MemInfo,
    uptime: u64,
    net: (u64, u64),
    disk_usage: DiskUsage,
    follows_ended: usize,
    ufw: Option<Vec<FirewallRule>>,
    ufw_pending: Vec<FirewallRule>,
    jails: Vec<String>,
    bans: Vec<Ban>,
    sshd: Sshd,
    keys: Vec<KeyFingerprint>,
    self_check: String,
    installed: Option<Vec<u8>>,
}

pub struct FakePlatform {
    inner: Mutex<Inner>,
}

/// Holds a matching call until `open` is called, so a test can watch what waits behind it.
pub struct Gate(Latch);

impl Gate {
    pub fn open(&self) {
        let (opened, wake) = &*self.0;
        *opened.lock().unwrap() = true;
        wake.notify_all();
    }
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
                disk_free: 50 * 1024 * 1024 * 1024,
                head: FAKE_HEAD.into(),
                proc_stat: ProcStat {
                    busy_ticks: 1000,
                    total_ticks: 4000,
                },
                meminfo: MemInfo {
                    total_kb: 2_097_152,
                    available_kb: 1_048_576,
                    swap_total_kb: 0,
                    swap_free_kb: 0,
                },
                uptime: 3600,
                disk_usage: DiskUsage {
                    used_bytes: 20 * 1024 * 1024 * 1024,
                    total_bytes: 80 * 1024 * 1024 * 1024,
                },
                sshd: Sshd {
                    port: 22,
                    password_auth: true,
                },
                self_check: "ferrum 0.1.4 (build fake, commit fake)".into(),
                ..Inner::default()
            }),
        }
    }

    pub fn journal(&self, unit: &str, lines: &[(u8, &str)]) {
        let mut inner = self.inner.lock().unwrap();
        let entries = inner.journal.entry(unit.to_string()).or_default();
        let mut at_usec = 1_788_348_289_288_518 + entries.len() as u64 * 1_000_000;
        for (priority, message) in lines {
            entries.push(JournalLine {
                at_usec,
                priority: *priority,
                message: (*message).to_string(),
            });
            at_usec += 1_000_000;
        }
    }

    pub fn set_cgroup(&self, unit: &str, stats: CgroupStats) {
        self.inner
            .lock()
            .unwrap()
            .cgroups
            .insert(unit.to_string(), stats);
    }

    pub fn clear_cgroup(&self, unit: &str) {
        self.inner.lock().unwrap().cgroups.remove(unit);
    }

    pub fn set_proc_stat(&self, stat: ProcStat) {
        self.inner.lock().unwrap().proc_stat = stat;
    }

    pub fn set_meminfo(&self, info: MemInfo) {
        let mut inner = self.inner.lock().unwrap();
        inner.meminfo = info;
        inner.memory_kb = info.total_kb;
        inner.swap_kb = info.swap_total_kb;
    }

    pub fn set_net(&self, rx: u64, tx: u64) {
        self.inner.lock().unwrap().net = (rx, tx);
    }

    pub fn set_disk_usage(&self, usage: DiskUsage) {
        self.inner.lock().unwrap().disk_usage = usage;
    }

    pub fn follows_ended(&self) -> usize {
        self.inner.lock().unwrap().follows_ended
    }

    pub fn set_ufw(&self, rules: Option<Vec<FirewallRule>>) {
        self.inner.lock().unwrap().ufw = rules;
    }

    pub fn set_jails(&self, jails: &[&str]) {
        self.inner.lock().unwrap().jails = jails.iter().map(|j| j.to_string()).collect();
    }

    pub fn ban(&self, jail: &str, ip: &str) {
        self.inner.lock().unwrap().bans.push(Ban {
            ip: ip.to_string(),
            jail: jail.to_string(),
            banned_at: Some("2026-09-02T10:12:33Z".into()),
        });
    }

    pub fn set_sshd(&self, sshd: Sshd) {
        self.inner.lock().unwrap().sshd = sshd;
    }

    pub fn add_key(&self, comment: &str) {
        let mut inner = self.inner.lock().unwrap();
        let n = inner.keys.len();
        inner.keys.push(KeyFingerprint {
            bits: 256,
            fingerprint: format!("SHA256:fake{n}"),
            comment: comment.to_string(),
            kind: "ED25519".into(),
        });
    }

    pub fn script_run(&self, contains: &str, lines: &[&str], exit: Exit) {
        self.inner.lock().unwrap().scripts.push((
            contains.to_string(),
            lines.iter().map(|l| l.to_string()).collect(),
            exit,
        ));
    }

    pub fn gate(&self, contains: &str) -> Gate {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        self.inner
            .lock()
            .unwrap()
            .gates
            .push((contains.to_string(), gate.clone()));
        Gate(gate)
    }

    pub fn runs(&self) -> Vec<RunSpec> {
        self.inner.lock().unwrap().runs.clone()
    }

    pub fn set_head(&self, sha: &str) {
        self.inner.lock().unwrap().head = sha.to_string();
    }

    pub fn set_disk_free(&self, bytes: u64) {
        self.inner.lock().unwrap().disk_free = bytes;
    }

    pub fn link(&self, path: &str) -> Option<String> {
        self.inner.lock().unwrap().links.get(path).cloned()
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

    pub fn sql(&self) -> Vec<String> {
        self.inner.lock().unwrap().sql.clone()
    }

    pub fn answer_sql(&self, contains: &str, output: &str) {
        self.inner
            .lock()
            .unwrap()
            .answers
            .push((contains.to_string(), output.to_string()));
    }

    pub fn set_postgres_major(&self, major: u32) {
        self.inner.lock().unwrap().postgres_major = Some(major);
    }

    pub fn answer_self_check(&self, line: &str) {
        self.inner.lock().unwrap().self_check = line.to_string();
    }

    /// The bytes the last `install_binary` read from its source.
    pub fn installed_binary(&self) -> Option<Vec<u8>> {
        self.inner.lock().unwrap().installed.clone()
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
        let gate = inner
            .gates
            .iter()
            .find(|(needle, _)| call.contains(needle.as_str()))
            .map(|(_, gate)| gate.clone());
        inner.calls.push(call);
        drop(inner);
        if let Some(gate) = gate {
            wait_for(&gate);
        }
        Ok(())
    }
}

fn wait_for(gate: &Latch) {
    let (opened, wake) = &**gate;
    let mut open = opened.lock().unwrap();
    while !*open {
        open = wake.wait(open).unwrap();
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

    fn read_file(&self, path: &Path) -> Result<Option<String>, PlatformError> {
        Ok(self.written(&path.to_string_lossy()))
    }

    fn file_exists(&self, path: &Path) -> bool {
        let p = path.to_string_lossy().to_string();
        let inner = self.inner.lock().unwrap();
        inner.files.contains_key(&p) || inner.dirs.contains(&p) || inner.links.contains_key(&p)
    }

    fn remove_file(&self, path: &Path) -> Result<(), PlatformError> {
        let p = path.to_string_lossy().to_string();
        self.record(format!("remove_file {p}"))?;
        self.inner.lock().unwrap().files.remove(&p);
        Ok(())
    }

    fn make_dirs(&self, path: &Path, mode: u32) -> Result<(), PlatformError> {
        let p = path.to_string_lossy().to_string();
        self.record(format!("make_dirs {p} {mode:o}"))?;
        self.inner.lock().unwrap().dirs.insert(p);
        Ok(())
    }

    fn remove_tree(&self, path: &Path) -> Result<(), PlatformError> {
        let p = path.to_string_lossy().to_string();
        self.record(format!("remove_tree {p}"))?;
        let mut inner = self.inner.lock().unwrap();
        let under = format!("{p}/");
        inner.files.retain(|k, _| !k.starts_with(&under));
        inner.links.retain(|k, _| !k.starts_with(&under));
        inner.dirs.retain(|k| k != &p && !k.starts_with(&under));
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

    fn extract_zip(
        &self,
        archive: &[u8],
        dest: &Path,
        strip_components: u32,
    ) -> Result<(), PlatformError> {
        self.record(format!(
            "extract_zip {} {strip_components}",
            dest.to_string_lossy()
        ))?;
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

    fn postgres_sql(&self, database: &str, sql: &str) -> Result<String, PlatformError> {
        self.record(format!("postgres_sql {database} {sql}"))?;
        let mut inner = self.inner.lock().unwrap();
        inner.sql.push(sql.to_string());
        Ok(inner
            .answers
            .iter()
            .find(|(needle, _)| sql.contains(needle.as_str()))
            .map(|(_, out)| out.clone())
            .unwrap_or_default())
    }

    fn postgres_major_installed(&self) -> Option<u32> {
        self.inner.lock().unwrap().postgres_major
    }

    fn postgres_dump(&self, database: &str, to: &Path) -> Result<(), PlatformError> {
        let p = to.to_string_lossy().to_string();
        self.record(format!("postgres_dump {database} {p}"))?;
        self.inner.lock().unwrap().files.insert(p, String::new());
        Ok(())
    }

    fn postgres_restore(&self, database: &str, from: &Path) -> Result<(), PlatformError> {
        self.record(format!(
            "postgres_restore {database} {}",
            from.to_string_lossy()
        ))
    }

    fn postgres_restore_sql(&self, database: &str, from: &Path) -> Result<(), PlatformError> {
        self.record(format!(
            "postgres_restore_sql {database} {}",
            from.to_string_lossy()
        ))
    }

    fn git_clone(
        &self,
        url: &str,
        git_ref: Option<&str>,
        dest: &Path,
        depth: u32,
    ) -> Result<(), PlatformError> {
        let dest = dest.to_string_lossy().to_string();
        self.record(format!(
            "git_clone {} {} {dest} {depth}",
            crate::ubuntu::redact_url(url, url),
            git_ref.unwrap_or("HEAD")
        ))?;
        let mut inner = self.inner.lock().unwrap();
        inner.dirs.insert(dest.clone());
        inner
            .files
            .insert(format!("{dest}/.git/HEAD"), "ref: refs/heads/main".into());
        Ok(())
    }

    fn git_checkout(&self, dir: &Path, commit_sha: &str) -> Result<(), PlatformError> {
        self.record(format!(
            "git_checkout {} {commit_sha}",
            dir.to_string_lossy()
        ))?;
        self.inner.lock().unwrap().head = commit_sha.to_string();
        Ok(())
    }

    fn git_head(&self, dir: &Path) -> Result<String, PlatformError> {
        self.record(format!("git_head {}", dir.to_string_lossy()))?;
        Ok(self.inner.lock().unwrap().head.clone())
    }

    fn git_scrub_remote(&self, dir: &Path, public_url: &str) -> Result<(), PlatformError> {
        self.record(format!(
            "git_scrub_remote {} {public_url}",
            dir.to_string_lossy()
        ))
    }

    fn run_scoped(
        &self,
        spec: &RunSpec,
        on_line: &mut dyn FnMut(Stream, &str),
    ) -> Result<Exit, PlatformError> {
        self.record(format!(
            "run_scoped {} {} {} MemoryMax={} {}",
            spec.unit,
            spec.user,
            spec.cwd.to_string_lossy(),
            spec.memory_max_mb,
            spec.command
        ))?;
        let (script, gate) = {
            let mut inner = self.inner.lock().unwrap();
            inner.runs.push(spec.clone());
            let script = inner
                .scripts
                .iter()
                .rev()
                .find(|(needle, _, _)| spec.command.contains(needle.as_str()))
                .map(|(_, lines, exit)| (lines.clone(), exit.clone()));
            let gate = inner
                .gates
                .iter()
                .find(|(needle, _)| spec.command.contains(needle.as_str()))
                .map(|(_, gate)| gate.clone());
            (script, gate)
        };
        if let Some(gate) = gate {
            wait_for(&gate);
        }
        let (lines, exit) = script.unwrap_or((Vec::new(), Exit::Code(0)));
        for line in &lines {
            on_line(Stream::Stdout, line);
        }
        Ok(exit)
    }

    fn symlink_swap(&self, target: &Path, link: &Path) -> Result<(), PlatformError> {
        let target = target.to_string_lossy().to_string();
        let link = link.to_string_lossy().to_string();
        self.record(format!("symlink_swap {target} {link}"))?;
        self.inner.lock().unwrap().links.insert(link, target);
        Ok(())
    }

    fn read_link(&self, link: &Path) -> Result<Option<PathBuf>, PlatformError> {
        Ok(self.link(&link.to_string_lossy()).map(PathBuf::from))
    }

    fn list_dir(&self, dir: &Path) -> Result<Vec<String>, PlatformError> {
        let prefix = format!("{}/", dir.to_string_lossy());
        let inner = self.inner.lock().unwrap();
        let mut names: Vec<String> = inner
            .files
            .keys()
            .chain(inner.links.keys())
            .chain(inner.dirs.iter())
            .filter_map(|p| p.strip_prefix(&prefix))
            .map(|rest| rest.split('/').next().unwrap_or(rest).to_string())
            .collect();
        names.sort();
        names.dedup();
        Ok(names)
    }

    fn disk_free_bytes(&self, path: &Path) -> Result<u64, PlatformError> {
        self.record(format!("disk_free_bytes {}", path.to_string_lossy()))?;
        Ok(self.inner.lock().unwrap().disk_free)
    }

    fn journal_tail(&self, unit: &str, lines: u32) -> Result<Vec<JournalLine>, PlatformError> {
        self.record(format!("journal_tail {unit} {lines}"))?;
        let inner = self.inner.lock().unwrap();
        let all = inner.journal.get(unit).cloned().unwrap_or_default();
        let skip = all.len().saturating_sub(lines as usize);
        Ok(all.into_iter().skip(skip).collect())
    }

    fn journal_follow(
        &self,
        unit: &str,
        lines: u32,
        on_line: &mut dyn FnMut(JournalLine),
        stopped: &dyn Fn() -> bool,
    ) -> Result<(), PlatformError> {
        self.record(format!("journal_follow {unit} {lines}"))?;
        let mut seen: Option<usize> = None;
        loop {
            let pending: Vec<JournalLine> = {
                let inner = self.inner.lock().unwrap();
                let all = inner.journal.get(unit).cloned().unwrap_or_default();
                let from = seen.unwrap_or_else(|| all.len().saturating_sub(lines as usize));
                let fresh = all.into_iter().skip(from).collect::<Vec<_>>();
                seen = Some(from + fresh.len());
                fresh
            };
            for line in pending {
                on_line(line);
            }
            if stopped() {
                break;
            }
            std::thread::sleep(FOLLOW_POLL);
        }
        self.inner.lock().unwrap().follows_ended += 1;
        Ok(())
    }

    fn cgroup_stats(&self, unit: &str) -> Result<Option<CgroupStats>, PlatformError> {
        Ok(self.inner.lock().unwrap().cgroups.get(unit).copied())
    }

    fn proc_stat(&self) -> Result<ProcStat, PlatformError> {
        Ok(self.inner.lock().unwrap().proc_stat)
    }

    fn proc_meminfo(&self) -> Result<MemInfo, PlatformError> {
        Ok(self.inner.lock().unwrap().meminfo)
    }

    fn uptime_secs(&self) -> Result<u64, PlatformError> {
        Ok(self.inner.lock().unwrap().uptime)
    }

    fn cpu_count(&self) -> usize {
        2
    }

    fn net_bytes(&self) -> Result<(u64, u64), PlatformError> {
        Ok(self.inner.lock().unwrap().net)
    }

    fn disk_usage(&self, path: &Path) -> Result<DiskUsage, PlatformError> {
        self.record(format!("disk_usage {}", path.to_string_lossy()))?;
        Ok(self.inner.lock().unwrap().disk_usage)
    }

    fn tail_file(&self, path: &Path, lines: u32) -> Result<Vec<String>, PlatformError> {
        let all: Vec<String> = self
            .written(&path.to_string_lossy())
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        let skip = all.len().saturating_sub(lines as usize);
        Ok(all.into_iter().skip(skip).collect())
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

    fn ufw_status(&self) -> Result<Option<Vec<FirewallRule>>, PlatformError> {
        self.record("ufw_status".into())?;
        Ok(self.inner.lock().unwrap().ufw.clone())
    }

    fn ufw_apply(&self, allow: &[&str]) -> Result<(), PlatformError> {
        self.record(format!("ufw_apply {}", allow.join(" ")))?;
        let rules: Vec<FirewallRule> = allow
            .iter()
            .map(|port| FirewallRule {
                port: port.to_string(),
                action: "allow".into(),
                from: "Anywhere".into(),
            })
            .collect();
        let mut inner = self.inner.lock().unwrap();
        if inner.ufw.is_some() {
            inner.ufw = Some(rules.clone());
        }
        inner.ufw_pending = rules;
        Ok(())
    }

    fn ufw_enable(&self) -> Result<(), PlatformError> {
        self.record("ufw_enable".into())?;
        let mut inner = self.inner.lock().unwrap();
        let rules = inner.ufw_pending.clone();
        inner.ufw = Some(rules);
        Ok(())
    }

    fn iptables_restore(&self, rules: &str) -> Result<(), PlatformError> {
        self.record(format!("iptables_restore {} lines", rules.lines().count()))
    }

    fn iptables_flush(&self) -> Result<(), PlatformError> {
        self.record("iptables_flush".into())
    }

    fn fail2ban_jails(&self) -> Result<Vec<String>, PlatformError> {
        self.record("fail2ban_jails".into())?;
        Ok(self.inner.lock().unwrap().jails.clone())
    }

    fn fail2ban_bans(&self, jail: &str) -> Result<Vec<Ban>, PlatformError> {
        self.record(format!("fail2ban_bans {jail}"))?;
        let inner = self.inner.lock().unwrap();
        Ok(inner
            .bans
            .iter()
            .filter(|b| b.jail == jail)
            .cloned()
            .collect())
    }

    fn fail2ban_unban(&self, jail: &str, ip: &str) -> Result<(), PlatformError> {
        self.record(format!("fail2ban_unban {jail} {ip}"))?;
        self.inner
            .lock()
            .unwrap()
            .bans
            .retain(|b| !(b.jail == jail && b.ip == ip));
        Ok(())
    }

    fn sshd_effective(&self) -> Result<Sshd, PlatformError> {
        self.record("sshd_effective".into())?;
        Ok(self.inner.lock().unwrap().sshd)
    }

    fn sshd_test(&self) -> Result<(), PlatformError> {
        self.record("sshd_test".into())
    }

    fn authorized_keys(&self) -> Result<Vec<KeyFingerprint>, PlatformError> {
        self.record("authorized_keys".into())?;
        Ok(self.inner.lock().unwrap().keys.clone())
    }

    fn self_check(&self, binary: &Path) -> Result<String, PlatformError> {
        self.record(format!("self_check {}", binary.display()))?;
        Ok(self.inner.lock().unwrap().self_check.clone())
    }

    fn install_binary(&self, from: &Path, to: &Path) -> Result<(), PlatformError> {
        self.record(format!(
            "install_binary {} {}",
            from.display(),
            to.display()
        ))?;
        let mut inner = self.inner.lock().unwrap();
        inner.installed = Some(std::fs::read(from)?);
        Ok(())
    }

    fn restart_later(&self, unit: &str) -> Result<(), PlatformError> {
        self.record(format!("restart_later {unit}"))
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
    fn sql_is_recorded_verbatim_and_can_be_answered_or_failed() {
        let p = FakePlatform::new();
        p.answer_sql("SELECT 1", "1\n");
        assert_eq!(p.postgres_sql("postgres", "SELECT 1;").unwrap(), "1\n");
        assert_eq!(p.postgres_sql("postgres", "SELECT 2;").unwrap(), "");
        assert_eq!(p.calls_matching("postgres_sql postgres").len(), 2);
        assert_eq!(
            p.sql(),
            vec!["SELECT 1;".to_string(), "SELECT 2;".to_string()]
        );
        assert_eq!(p.postgres_major_installed(), None);
        p.set_postgres_major(18);
        assert_eq!(p.postgres_major_installed(), Some(18));
        p.fail_next("DROP DATABASE");
        assert!(p.postgres_sql("postgres", "DROP DATABASE \"x\";").is_err());
    }

    fn spec(command: &str) -> RunSpec {
        RunSpec {
            unit: "ferrum-build-ledger-1".into(),
            user: "ferrum-ledger".into(),
            cwd: PathBuf::from("/var/lib/ferrum/apps/ledger/releases/r1"),
            command: command.into(),
            env: Vec::new(),
            memory_max_mb: 1200,
            cpu_weight: 50,
            io_weight: 50,
            timeout: std::time::Duration::from_secs(60),
        }
    }

    #[test]
    fn a_scoped_run_plays_scripted_output_and_exit_and_records_the_spec() {
        let p = FakePlatform::new();
        p.script_run("bun run build", &["building…", "done"], Exit::Code(0));
        let mut seen = vec![];
        let exit = p
            .run_scoped(&spec("bun run build"), &mut |_, l| seen.push(l.to_string()))
            .unwrap();
        assert_eq!(exit, Exit::Code(0));
        assert_eq!(seen, vec!["building…", "done"]);
        assert!(p.calls().iter().any(
            |c| c.starts_with("run_scoped ferrum-build-ledger") && c.contains("MemoryMax=1200")
        ));
        assert_eq!(p.runs()[0].command, "bun run build");
        p.script_run(
            "bun run build",
            &["FATAL ERROR: heap out of memory"],
            Exit::Killed { signal: 9 },
        );
        assert_eq!(
            p.run_scoped(&spec("bun run build"), &mut |_, _| {})
                .unwrap(),
            Exit::Killed { signal: 9 }
        );
        assert_eq!(
            p.run_scoped(&spec("something else"), &mut |_, _| {})
                .unwrap(),
            Exit::Code(0)
        );
    }

    #[test]
    fn symlinks_listings_and_clones_are_kept_in_the_fake_filesystem() {
        let p = FakePlatform::new();
        p.symlink_swap(Path::new("/a/releases/r1"), Path::new("/a/current"))
            .unwrap();
        assert_eq!(
            p.read_link(Path::new("/a/current")).unwrap(),
            Some(PathBuf::from("/a/releases/r1"))
        );
        p.write_file(Path::new("/a/releases/r1/x"), "", 0o644)
            .unwrap();
        p.git_clone(
            "https://x-access-token:ghs_abc@github.com/irixsoft/ledger.git",
            Some("main"),
            Path::new("/a/releases/r2"),
            1,
        )
        .unwrap();
        assert!(p.file_exists(Path::new("/a/releases/r2/.git/HEAD")));
        assert!(p.file_exists(Path::new("/a/releases/r2")));
        assert!(
            p.calls()
                .iter()
                .any(|c| c
                    == "git_clone https://github.com/irixsoft/ledger.git main /a/releases/r2 1")
        );
        assert!(!p.calls().join("\n").contains("ghs_abc"));
        assert_eq!(
            p.list_dir(Path::new("/a/releases")).unwrap(),
            vec!["r1", "r2"]
        );
        p.remove_tree(Path::new("/a/releases/r1")).unwrap();
        assert_eq!(p.list_dir(Path::new("/a/releases")).unwrap(), vec!["r2"]);
        assert_eq!(p.git_head(Path::new("/a/releases/r2")).unwrap().len(), 40);
        p.set_head("a3f9c2d4e81b06f5c9a2");
        assert_eq!(
            p.git_head(Path::new("/a/releases/r2")).unwrap(),
            "a3f9c2d4e81b06f5c9a2"
        );
        p.set_disk_free(200);
        assert_eq!(p.disk_free_bytes(Path::new("/a")).unwrap(), 200);
    }

    #[test]
    fn a_gate_holds_a_scripted_command_until_it_is_opened() {
        let p = Arc::new(FakePlatform::new());
        let gate = p.gate("bun run build");
        let worker = {
            let p = p.clone();
            std::thread::spawn(move || p.run_scoped(&spec("bun run build"), &mut |_, _| {}))
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!worker.is_finished(), "the command must wait for the gate");
        gate.open();
        assert_eq!(worker.join().unwrap().unwrap(), Exit::Code(0));
    }

    #[test]
    fn a_gate_holds_any_recorded_call_until_it_is_opened() {
        let p = Arc::new(FakePlatform::new());
        let gate = p.gate("install_packages ufw");
        let worker = {
            let p = p.clone();
            std::thread::spawn(move || p.install_packages(&["ufw"]))
        };
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!worker.is_finished(), "the call must wait for the gate");
        assert_eq!(p.calls_matching("install_packages").len(), 1);
        gate.open();
        worker.join().unwrap().unwrap();
    }

    #[test]
    fn memory_and_swap_are_settable() {
        let p = FakePlatform::new();
        p.set_memory_kb(2_048_000);
        p.set_swap_kb(0);
        assert_eq!(p.total_memory_kb().unwrap(), 2_048_000);
        assert_eq!(p.swap_total_kb().unwrap(), 0);
    }

    #[test]
    fn scripted_journal_lines_are_tailed_and_a_follow_ends_when_told_to() {
        let p = Arc::new(FakePlatform::new());
        p.journal(
            "ferrum-app-ledger",
            &[(6, "Listening on 41204"), (4, "slow query"), (3, "boom")],
        );
        let tail = p.journal_tail("ferrum-app-ledger", 2).unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].message, "slow query");
        assert!(tail[1].at_usec > tail[0].at_usec);
        assert!(p.journal_tail("nope", 5).unwrap().is_empty());

        let stop = Arc::new(Mutex::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let worker = {
            let (p, stop, seen) = (p.clone(), stop.clone(), seen.clone());
            std::thread::spawn(move || {
                p.journal_follow(
                    "ferrum-app-ledger",
                    1,
                    &mut |line| seen.lock().unwrap().push(line.message),
                    &|| *stop.lock().unwrap(),
                )
            })
        };
        std::thread::sleep(Duration::from_millis(30));
        p.journal("ferrum-app-ledger", &[(6, "later")]);
        std::thread::sleep(Duration::from_millis(30));
        assert!(!worker.is_finished());
        *stop.lock().unwrap() = true;
        worker.join().unwrap().unwrap();
        assert_eq!(*seen.lock().unwrap(), vec!["boom", "later"]);
        assert_eq!(p.follows_ended(), 1);
    }

    #[test]
    fn the_hardening_tools_are_scripted_and_recorded() {
        let p = FakePlatform::new();
        assert_eq!(p.ufw_status().unwrap(), None);
        p.ufw_apply(&["2222/tcp", "80/tcp"]).unwrap();
        assert_eq!(p.ufw_status().unwrap(), None, "no enable, still inactive");
        p.ufw_enable().unwrap();
        let rules = p.ufw_status().unwrap().unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].port, "2222/tcp");
        assert!(p.calls().contains(&"ufw_apply 2222/tcp 80/tcp".to_string()));
        p.iptables_restore("*filter\nCOMMIT\n").unwrap();
        p.iptables_flush().unwrap();
        assert_eq!(
            p.calls_matching("iptables_"),
            vec!["iptables_restore 2 lines", "iptables_flush"]
        );

        assert!(p.fail2ban_jails().unwrap().is_empty());
        p.set_jails(&["sshd", "nginx-botsearch"]);
        p.ban("sshd", "45.148.10.87");
        p.ban("nginx-botsearch", "185.220.101.4");
        assert_eq!(p.fail2ban_bans("sshd").unwrap().len(), 1);
        p.fail2ban_unban("sshd", "45.148.10.87").unwrap();
        assert!(p.fail2ban_bans("sshd").unwrap().is_empty());
        assert_eq!(p.fail2ban_bans("nginx-botsearch").unwrap().len(), 1);

        assert_eq!(p.sshd_effective().unwrap().port, 22);
        p.set_sshd(Sshd {
            port: 2222,
            password_auth: false,
        });
        assert!(!p.sshd_effective().unwrap().password_auth);
        assert!(p.authorized_keys().unwrap().is_empty());
        p.add_key("saeed@laptop");
        assert_eq!(p.authorized_keys().unwrap()[0].comment, "saeed@laptop");
        p.fail_next("sshd_test");
        assert!(p.sshd_test().is_err());
    }

    #[test]
    fn host_readings_are_settable_and_a_missing_cgroup_is_none() {
        let p = FakePlatform::new();
        assert_eq!(p.cgroup_stats("ferrum-app-ledger").unwrap(), None);
        p.set_cgroup(
            "ferrum-app-ledger",
            CgroupStats {
                memory_current: 10,
                memory_peak: 20,
                cpu_usage_usec: 30,
            },
        );
        assert_eq!(
            p.cgroup_stats("ferrum-app-ledger")
                .unwrap()
                .unwrap()
                .memory_peak,
            20
        );
        p.clear_cgroup("ferrum-app-ledger");
        assert_eq!(p.cgroup_stats("ferrum-app-ledger").unwrap(), None);
        p.set_net(5, 7);
        assert_eq!(p.net_bytes().unwrap(), (5, 7));
        p.write_file(Path::new("/var/log/nginx/x.log"), "a\nb\nc\n", 0o644)
            .unwrap();
        assert_eq!(
            p.tail_file(Path::new("/var/log/nginx/x.log"), 2).unwrap(),
            vec!["b", "c"]
        );
        assert!(p.tail_file(Path::new("/nope"), 2).unwrap().is_empty());
        assert_eq!(p.proc_meminfo().unwrap().total_kb, 2_097_152);
        assert_eq!(p.cpu_count(), 2);
    }
}
