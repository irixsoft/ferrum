use super::{
    ArchiveFormat, Commands, Detection, Health, PackageManager, Phase, Runtime, RuntimeKind,
    Source, Target, node, path_with, semver_like, version_prefix,
};
use crate::detect::RepoTree;
use crate::github::Api;
use anyhow::Context;
use ferrum_platform::Arch;
use serde::Deserialize;
use std::path::Path;

const RELEASES: &str = "https://github.com/oven-sh/bun/releases/download";
pub const LATEST_ROUTE: &str = "/repos/oven-sh/bun/releases/latest";
const TAG_PREFIX: &str = "bun-v";

pub struct Bun;

pub fn arch_name(arch: Arch) -> &'static str {
    match arch {
        Arch::X86_64 => "x64",
        Arch::Aarch64 => "aarch64",
    }
}

pub fn download_url(version: &str, target: Target) -> String {
    let arch = arch_name(target.arch);
    let flavour = if target.baseline { "-baseline" } else { "" };
    format!("{RELEASES}/bun-v{version}/bun-linux-{arch}{flavour}.zip")
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

pub async fn latest(api: &Api) -> anyhow::Result<String> {
    let release: Release = api
        .anonymous()?
        .get(LATEST_ROUTE, None::<&()>)
        .await
        .context("asking github for the latest bun release")?;
    release
        .tag_name
        .strip_prefix(TAG_PREFIX)
        .map(str::to_string)
        .with_context(|| format!("unexpected bun release tag {}", release.tag_name))
}

pub async fn resolve(api: &Api, wanted: Option<&str>) -> anyhow::Result<String> {
    match wanted {
        Some(w) if semver_like(w, 3) => Ok(w.to_string()),
        Some(w) => {
            let newest = latest(api).await?;
            if newest.starts_with(&format!("{w}.")) || newest == w {
                Ok(newest)
            } else {
                anyhow::bail!(
                    "Bun {w} is not the current release ({newest}); enter the full version"
                )
            }
        }
        None => latest(api).await,
    }
}

fn wanted_version(tree: &RepoTree, package: &serde_json::Value) -> Option<(String, String)> {
    if let Some(raw) = tree.read(".bun-version")
        && let Some(v) = version_prefix(raw)
    {
        return Some((v, format!("Bun {} from .bun-version", raw.trim())));
    }
    if let Some(spec) = package["packageManager"]
        .as_str()
        .and_then(|s| s.strip_prefix("bun@"))
        && let Some(v) = version_prefix(spec)
    {
        return Some((v, format!("Bun {spec} from packageManager in package.json")));
    }
    if let Some(spec) = package["engines"]["bun"].as_str()
        && let Some(v) = version_prefix(spec)
    {
        return Some((v, format!("Bun {spec} from engines in package.json")));
    }
    None
}

impl Runtime for Bun {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Bun
    }

    fn detect(&self, tree: &RepoTree) -> Option<Detection> {
        let package = tree.json("package.json")?;
        let (pm, lock) = node::package_manager(tree)?;
        if pm != PackageManager::Bun {
            return None;
        }
        let mut reasons = vec!["found package.json".to_string(), format!("found {lock}")];
        let (has_build, has_start) = node::scripts(&package);
        let start_script = package["scripts"]["start"].as_str().unwrap_or("");
        let runs_on_bun = start_script.starts_with("bun ");

        let confidence = if runs_on_bun {
            reasons.push("the start script runs bun".to_string());
            85
        } else if node::framework(tree).is_some() {
            30
        } else {
            60
        };

        let version = wanted_version(tree, &package).map(|(v, why)| {
            reasons.push(why);
            v
        });

        let start = if has_start {
            Some(pm.run("start"))
        } else {
            package["main"].as_str().map(|main| format!("bun {main}"))
        };

        Some(Detection {
            kind: RuntimeKind::Bun,
            toolchain: RuntimeKind::Bun,
            version,
            confidence,
            reasons,
            commands: Commands {
                install: Some(pm.install(true).to_string()),
                build: has_build.then(|| pm.run("build")),
                start,
                migrate: None,
            },
            output_dir: None,
            health: Health::default(),
            package_manager: Some(pm),
        })
    }

    fn source(&self, version: &str, target: Target, _install_dir: &Path) -> Option<Source> {
        Some(Source::Archive {
            url: download_url(version, target),
            format: ArchiveFormat::Zip,
            strip_components: 1,
        })
    }

    fn binary(&self) -> &'static str {
        "bun"
    }

    fn valid_version(&self, version: &str) -> bool {
        semver_like(version, 3)
    }

    fn env_for(&self, phase: Phase, toolchain: &Path, _port: Option<u16>) -> Vec<(String, String)> {
        let mut env = vec![path_with(toolchain)];
        if phase == Phase::Run {
            env.push(("NODE_ENV".into(), "production".into()));
        }
        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn download_urls_name_the_right_architecture_and_fall_back_to_baseline() {
        let arm = Target {
            arch: Arch::Aarch64,
            baseline: false,
        };
        assert_eq!(
            download_url("1.2.3", arm),
            "https://github.com/oven-sh/bun/releases/download/bun-v1.2.3/bun-linux-aarch64.zip"
        );
        let old_x64 = Target {
            arch: Arch::X86_64,
            baseline: true,
        };
        assert_eq!(
            download_url("1.2.3", old_x64),
            "https://github.com/oven-sh/bun/releases/download/bun-v1.2.3/bun-linux-x64-baseline.zip"
        );
    }

    #[test]
    fn a_bun_start_script_makes_bun_the_runtime() {
        let tree = RepoTree::from_files(&[
            (
                "package.json",
                r#"{"scripts":{"start":"bun run src/index.ts"},"packageManager":"bun@1.2.3"}"#,
            ),
            ("bun.lock", ""),
        ]);
        let d = Bun.detect(&tree).unwrap();
        assert_eq!(d.confidence, 85);
        assert_eq!(d.version.as_deref(), Some("1.2.3"));
        assert_eq!(d.commands.start.as_deref(), Some("bun run start"));
        assert!(d.commands.build.is_none());
    }

    #[test]
    fn a_nextjs_app_with_a_bun_lockfile_is_not_a_bun_app() {
        let tree = RepoTree::from_files(&[
            (
                "package.json",
                r#"{"scripts":{"build":"next build","start":"next start"}}"#,
            ),
            ("next.config.js", ""),
            ("bun.lockb", ""),
        ]);
        let bun = Bun.detect(&tree).unwrap().confidence;
        let node = node::Node.detect(&tree).unwrap().confidence;
        assert!(bun < node, "bun {bun} must rank below node {node}");
    }

    #[test]
    fn without_a_bun_lockfile_bun_stays_out() {
        let tree = RepoTree::from_files(&[("package.json", "{}"), ("package-lock.json", "")]);
        assert!(Bun.detect(&tree).is_none());
    }
}
