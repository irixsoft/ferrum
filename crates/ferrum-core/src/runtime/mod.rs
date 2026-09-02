pub mod bun;
pub mod dotnet;
pub mod node;
pub mod static_site;
pub mod toolchain;

use crate::detect::RepoTree;
use anyhow::Context;
use ferrum_platform::{Arch, Platform};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum RuntimeKind {
    Node,
    Bun,
    Static,
    Dotnet,
}

impl RuntimeKind {
    pub const ALL: [RuntimeKind; 4] = [Self::Node, Self::Bun, Self::Static, Self::Dotnet];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Node => "node",
            Self::Bun => "bun",
            Self::Static => "static",
            Self::Dotnet => "dotnet",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub fn has_process(self) -> bool {
        self != Self::Static
    }

    pub fn installs_toolchain(self) -> bool {
        self != Self::Static
    }
}

impl fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Build,
    Run,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    pub arch: Arch,
    /// x86_64 without AVX2: Bun's default build dies with an illegal instruction there.
    pub baseline: bool,
}

impl Target {
    pub fn of(platform: &dyn Platform) -> anyhow::Result<Self> {
        let arch = Arch::current().context("this CPU architecture is not supported")?;
        Ok(Self {
            arch,
            baseline: arch == Arch::X86_64 && !platform.cpu_has("avx2"),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageManager {
    Npm,
    Pnpm,
    Yarn,
    Bun,
}

impl PackageManager {
    pub fn install(self, locked: bool) -> &'static str {
        match (self, locked) {
            (Self::Npm, true) => "npm ci",
            (Self::Npm, false) => "npm install",
            (Self::Pnpm, _) => "pnpm install --frozen-lockfile",
            (Self::Yarn, _) => "yarn install --frozen-lockfile",
            (Self::Bun, _) => "bun install --frozen-lockfile",
        }
    }

    pub fn run(self, script: &str) -> String {
        let bin = match self {
            Self::Npm => "npm",
            Self::Pnpm => "pnpm",
            Self::Yarn => "yarn",
            Self::Bun => "bun",
        };
        format!("{bin} run {script}")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commands {
    pub install: Option<String>,
    pub build: Option<String>,
    pub start: Option<String>,
    pub migrate: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub path: String,
    pub startup_budget_secs: u32,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            path: "/".into(),
            startup_budget_secs: 60,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Detection {
    pub kind: RuntimeKind,
    pub toolchain: RuntimeKind,
    pub version: Option<String>,
    pub confidence: u8,
    pub reasons: Vec<String>,
    pub commands: Commands,
    pub output_dir: Option<String>,
    pub health: Health,
    pub package_manager: Option<PackageManager>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Archive {
        url: String,
        format: ArchiveFormat,
        strip_components: u32,
    },
    Script {
        url: String,
        args: Vec<String>,
        packages: &'static [&'static str],
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    TarGz,
    Zip,
}

pub trait Runtime: Send + Sync {
    fn kind(&self) -> RuntimeKind;
    fn detect(&self, tree: &RepoTree) -> Option<Detection>;
    fn source(&self, version: &str, target: Target, install_dir: &Path) -> Option<Source>;
    fn binary(&self) -> &'static str;
    fn valid_version(&self, version: &str) -> bool;
    fn env_for(&self, phase: Phase, toolchain: &Path, port: Option<u16>) -> Vec<(String, String)>;
}

pub fn all() -> [&'static dyn Runtime; 4] {
    [
        &node::Node,
        &bun::Bun,
        &static_site::Static,
        &dotnet::Dotnet,
    ]
}

pub fn by_kind(kind: RuntimeKind) -> &'static dyn Runtime {
    match kind {
        RuntimeKind::Node => &node::Node,
        RuntimeKind::Bun => &bun::Bun,
        RuntimeKind::Static => &static_site::Static,
        RuntimeKind::Dotnet => &dotnet::Dotnet,
    }
}

pub fn path_with(toolchain_bin: &Path) -> (String, String) {
    (
        "PATH".into(),
        format!(
            "{}:{}",
            toolchain_bin.display(),
            ferrum_platform::ubuntu::SYSTEM_PATH
        ),
    )
}

/// Digits and dots only, so "v22", ">=22.11" and "^1.2.3" all become a version prefix.
pub fn version_prefix(spec: &str) -> Option<String> {
    let trimmed = spec
        .trim()
        .trim_start_matches(['v', '^', '~', '>', '=', '<', ' ']);
    let digits: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let digits = digits.trim_matches('.').to_string();
    (!digits.is_empty()).then_some(digits)
}

pub fn semver_like(version: &str, parts: usize) -> bool {
    let pieces: Vec<&str> = version.split('.').collect();
    pieces.len() == parts
        && pieces
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn every_runtime_has_a_distinct_kind() {
        let kinds: HashSet<_> = all().iter().map(|r| r.kind()).collect();
        assert_eq!(kinds.len(), 4);
        for kind in RuntimeKind::ALL {
            assert_eq!(by_kind(kind).kind(), kind);
            assert_eq!(RuntimeKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn version_specs_become_prefixes() {
        assert_eq!(version_prefix(">=22").as_deref(), Some("22"));
        assert_eq!(version_prefix("v22.11.0\n").as_deref(), Some("22.11.0"));
        assert_eq!(version_prefix("^1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(version_prefix("lts/*"), None);
        assert_eq!(version_prefix("22.x").as_deref(), Some("22"));
    }

    #[test]
    fn full_versions_are_recognised() {
        assert!(semver_like("22.11.0", 3));
        assert!(!semver_like("22", 3));
        assert!(semver_like("9.0", 2));
        assert!(!semver_like("9.0.1", 2));
        assert!(!semver_like("a.b.c", 3));
    }

    #[test]
    fn only_static_sites_are_processless() {
        assert!(!RuntimeKind::Static.has_process());
        assert!(RuntimeKind::Node.has_process());
    }
}
