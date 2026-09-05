use super::{
    ArchiveFormat, Commands, Detection, Health, Mirrors, PackageManager, Phase, Runtime,
    RuntimeKind, Source, Target, path_with, semver_like, version_prefix,
};
use crate::detect::RepoTree;
use anyhow::Context;
use ferrum_platform::Arch;
use serde::Deserialize;
use std::path::Path;

pub const DIST: &str = "https://nodejs.org/dist";

const FRAMEWORK_CONFIGS: [&str; 7] = [
    "next.config.*",
    "nuxt.config.*",
    "svelte.config.*",
    "remix.config.*",
    "react-router.config.*",
    "nest-cli.json",
    "astro.config.*",
];

pub struct Node;

pub fn arch_name(arch: Arch) -> &'static str {
    match arch {
        Arch::X86_64 => "x64",
        Arch::Aarch64 => "arm64",
    }
}

pub fn download_url(dist: &str, version: &str, arch: Arch) -> String {
    let arch = arch_name(arch);
    format!("{dist}/v{version}/node-v{version}-linux-{arch}.tar.gz")
}

#[derive(Deserialize)]
struct IndexEntry {
    version: String,
    lts: serde_json::Value,
    files: Vec<String>,
}

/// Newest LTS when nothing is asked for, else the newest release under the asked-for prefix.
pub fn pick(index: &str, wanted: Option<&str>) -> anyhow::Result<String> {
    let entries: Vec<IndexEntry> =
        serde_json::from_str(index).context("nodejs.org's release index did not parse")?;
    let prefix = wanted.and_then(version_prefix);
    let chosen = entries
        .iter()
        .filter(|e| e.files.iter().any(|f| f == "linux-x64"))
        .find(|e| {
            let version = e.version.trim_start_matches('v');
            match &prefix {
                Some(p) => version == p || version.starts_with(&format!("{p}.")),
                None => e.lts != serde_json::Value::Bool(false),
            }
        })
        .with_context(|| match wanted {
            Some(w) => format!("nodejs.org has no release matching Node {w}"),
            None => "nodejs.org lists no LTS release".to_string(),
        })?;
    Ok(chosen.version.trim_start_matches('v').to_string())
}

pub async fn resolve(
    http: &reqwest::Client,
    index_url: &str,
    wanted: Option<&str>,
) -> anyhow::Result<String> {
    if let Some(w) = wanted
        && semver_like(w, 3)
    {
        return Ok(w.to_string());
    }
    let index = http
        .get(index_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .context("reading nodejs.org's release index")?
        .text()
        .await?;
    pick(&index, wanted)
}

pub fn package_manager(tree: &RepoTree) -> Option<(PackageManager, &'static str)> {
    [
        ("bun.lock", PackageManager::Bun),
        ("bun.lockb", PackageManager::Bun),
        ("pnpm-lock.yaml", PackageManager::Pnpm),
        ("yarn.lock", PackageManager::Yarn),
        ("package-lock.json", PackageManager::Npm),
    ]
    .into_iter()
    .find(|(lock, _)| tree.has(lock))
    .map(|(lock, pm)| (pm, lock))
}

pub fn wanted_version(tree: &RepoTree, package: &serde_json::Value) -> Option<(String, String)> {
    for file in [".nvmrc", ".node-version"] {
        if let Some(v) = tree.read(file).and_then(version_prefix) {
            return Some((v, format!("Node {} from {file}", short(tree.read(file)?))));
        }
    }
    if let Some(spec) = package["engines"]["node"].as_str()
        && let Some(v) = version_prefix(spec)
    {
        return Some((v, format!("Node {spec} from engines in package.json")));
    }
    if let Some(spec) = package["volta"]["node"].as_str()
        && let Some(v) = version_prefix(spec)
    {
        return Some((v, format!("Node {spec} from volta in package.json")));
    }
    None
}

fn short(raw: &str) -> &str {
    raw.trim()
}

pub fn scripts(package: &serde_json::Value) -> (bool, bool) {
    let scripts = &package["scripts"];
    (scripts["build"].is_string(), scripts["start"].is_string())
}

const MIGRATE_SCRIPTS: [&str; 3] = ["db:migrate", "migrate", "migrate:deploy"];
const MIGRATE_COMMANDS: [&str; 5] = [
    "drizzle-kit migrate",
    "prisma migrate deploy",
    "knex migrate:latest",
    "payload migrate",
    "typeorm migration:run",
];
const DB_CLIENTS: [&str; 10] = [
    "pg",
    "postgres",
    "drizzle-orm",
    "@prisma/client",
    "prisma",
    "knex",
    "mysql2",
    "better-sqlite3",
    "typeorm",
    "@payloadcms/db-postgres",
];

/// The first of `names` found under dependencies or devDependencies.
pub fn depends_on(package: &serde_json::Value, names: &[&str]) -> Option<String> {
    ["dependencies", "devDependencies"]
        .iter()
        .filter_map(|section| package[section].as_object())
        .find_map(|deps| names.iter().find(|n| deps.contains_key(**n)))
        .map(|n| n.to_string())
}

/// A migrate script is only proposed when a database client is a dependency; a bare
/// `migrate` script in a library repository is not one.
pub fn migrate_script(package: &serde_json::Value) -> Option<(String, String)> {
    let scripts = package["scripts"].as_object()?;
    depends_on(package, &DB_CLIENTS)?;
    let name = MIGRATE_SCRIPTS
        .iter()
        .find(|n| scripts.contains_key(**n))
        .map(|n| n.to_string())
        .or_else(|| {
            scripts
                .iter()
                .find(|(_, v)| {
                    v.as_str()
                        .is_some_and(|s| MIGRATE_COMMANDS.iter().any(|c| s.contains(c)))
                })
                .map(|(k, _)| k.clone())
        })?;
    let why = format!("found migrate script {name}");
    Some((name, why))
}

pub fn framework(tree: &RepoTree) -> Option<String> {
    FRAMEWORK_CONFIGS
        .iter()
        .find_map(|glob| tree.matching(glob).first().map(|p| p.to_string()))
}

impl Runtime for Node {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Node
    }

    fn detect(&self, tree: &RepoTree) -> Option<Detection> {
        let package = tree.json("package.json")?;
        let mut reasons = vec!["found package.json".to_string()];
        let (has_build, has_start) = scripts(&package);

        let (pm, locked) = match package_manager(tree) {
            Some((pm, lock)) => {
                reasons.push(format!("found {lock}"));
                (pm, true)
            }
            None => (PackageManager::Npm, false),
        };

        let mut confidence = if locked { 70 } else { 50 };
        if let Some(config) = framework(tree) {
            reasons.push(format!("found {config}"));
            confidence = 90;
        }
        if !has_start {
            confidence = confidence.min(40);
        }

        let version = wanted_version(tree, &package).map(|(v, why)| {
            reasons.push(why);
            v
        });

        let start = if has_start {
            Some(pm.run("start"))
        } else {
            package["main"].as_str().map(|main| format!("node {main}"))
        };
        let migrate = migrate_script(&package).map(|(name, why)| {
            reasons.push(why);
            pm.run(&name)
        });

        Some(Detection {
            kind: RuntimeKind::Node,
            toolchain: RuntimeKind::Node,
            version,
            confidence,
            reasons,
            commands: Commands {
                install: Some(pm.install(locked).to_string()),
                build: has_build.then(|| pm.run("build")),
                start,
                migrate,
            },
            output_dir: None,
            health: Health::default(),
            package_manager: Some(pm),
        })
    }

    fn source(
        &self,
        version: &str,
        target: Target,
        _install_dir: &Path,
        mirrors: &Mirrors,
    ) -> Option<Source> {
        Some(Source::Archive {
            url: download_url(&mirrors.node_dist, version, target.arch),
            format: ArchiveFormat::TarGz,
            strip_components: 1,
        })
    }

    fn binary(&self) -> &'static str {
        "bin/node"
    }

    fn valid_version(&self, version: &str) -> bool {
        semver_like(version, 3)
    }

    fn env_for(&self, phase: Phase, toolchain: &Path, _port: Option<u16>) -> Vec<(String, String)> {
        let mut env = vec![path_with(&toolchain.join("bin"))];
        if phase == Phase::Run {
            env.push(("NODE_ENV".into(), "production".into()));
        }
        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = r#"[
      {"version":"v26.8.1","date":"2026-08-26","files":["linux-arm64","linux-x64"],"lts":false},
      {"version":"v24.9.0","date":"2026-08-20","files":["linux-arm64","linux-x64"],"lts":"Krypton"},
      {"version":"v22.20.0","date":"2026-08-10","files":["linux-arm64","linux-x64"],"lts":"Jod"},
      {"version":"v22.11.0","date":"2024-10-29","files":["linux-arm64","linux-x64"],"lts":"Jod"},
      {"version":"v0.8.0","date":"2012-06-25","files":["src"],"lts":false}
    ]"#;

    #[test]
    fn download_urls_name_the_right_architecture() {
        assert_eq!(
            download_url(DIST, "22.11.0", Arch::Aarch64),
            "https://nodejs.org/dist/v22.11.0/node-v22.11.0-linux-arm64.tar.gz"
        );
        assert_eq!(
            download_url(DIST, "22.11.0", Arch::X86_64),
            "https://nodejs.org/dist/v22.11.0/node-v22.11.0-linux-x64.tar.gz"
        );
    }

    #[test]
    fn the_newest_lts_is_picked_when_nothing_is_asked_for() {
        assert_eq!(pick(INDEX, None).unwrap(), "24.9.0");
    }

    #[test]
    fn a_major_resolves_to_its_newest_release_and_an_exact_version_to_itself() {
        assert_eq!(pick(INDEX, Some("22")).unwrap(), "22.20.0");
        assert_eq!(pick(INDEX, Some(">=22")).unwrap(), "22.20.0");
        assert_eq!(pick(INDEX, Some("22.11.0")).unwrap(), "22.11.0");
        assert_eq!(pick(INDEX, Some("26")).unwrap(), "26.8.1");
        assert!(pick(INDEX, Some("2")).is_err(), "2 must not match 22 or 26");
        assert!(pick(INDEX, Some("99")).is_err());
    }

    #[test]
    fn a_nextjs_app_with_bun_lockfile_is_node_managed_by_bun() {
        let tree = RepoTree::from_files(&[
            (
                "package.json",
                r#"{"scripts":{"build":"next build","start":"next start"},"engines":{"node":">=22"}}"#,
            ),
            ("next.config.js", ""),
            ("bun.lockb", ""),
        ]);
        let d = Node.detect(&tree).unwrap();
        assert_eq!(
            d.commands.install.as_deref(),
            Some("bun install --frozen-lockfile")
        );
        assert_eq!(d.commands.build.as_deref(), Some("bun run build"));
        assert_eq!(d.commands.start.as_deref(), Some("bun run start"));
        assert_eq!(d.version.as_deref(), Some("22"));
        assert_eq!(d.confidence, 90);
        assert!(
            d.reasons.iter().any(|r| r.contains("next.config.js")),
            "{:?}",
            d.reasons
        );
        assert!(d.reasons.iter().any(|r| r.contains("bun.lockb")));
        assert_eq!(d.package_manager, Some(PackageManager::Bun));
    }

    #[test]
    fn the_lockfile_decides_the_package_manager() {
        for (lock, install) in [
            ("pnpm-lock.yaml", "pnpm install --frozen-lockfile"),
            ("yarn.lock", "yarn install --frozen-lockfile"),
            ("package-lock.json", "npm ci"),
            ("bun.lock", "bun install --frozen-lockfile"),
        ] {
            let tree = RepoTree::from_files(&[("package.json", "{}"), (lock, "")]);
            assert_eq!(
                Node.detect(&tree).unwrap().commands.install.as_deref(),
                Some(install),
                "{lock}"
            );
        }
        let tree = RepoTree::from_files(&[("package.json", "{}")]);
        assert_eq!(
            Node.detect(&tree).unwrap().commands.install.as_deref(),
            Some("npm install"),
            "no lockfile means no --frozen-lockfile"
        );
    }

    #[test]
    fn nvmrc_beats_engines() {
        let tree = RepoTree::from_files(&[
            ("package.json", r#"{"engines":{"node":"20"}}"#),
            (".nvmrc", "22.11.0\n"),
        ]);
        assert_eq!(
            Node.detect(&tree).unwrap().version.as_deref(),
            Some("22.11.0")
        );
    }

    #[test]
    fn a_migrate_script_is_proposed_only_alongside_a_database_client() {
        let drizzle = RepoTree::from_files(&[
            (
                "package.json",
                r#"{"scripts":{"start":"next start","db:migrate":"drizzle-kit migrate"},"dependencies":{"drizzle-orm":"1"}}"#,
            ),
            ("bun.lock", ""),
        ]);
        let d = Node.detect(&drizzle).unwrap();
        assert_eq!(d.commands.migrate.as_deref(), Some("bun run db:migrate"));
        assert!(
            d.reasons
                .iter()
                .any(|r| r == "found migrate script db:migrate"),
            "{:?}",
            d.reasons
        );

        let by_value = serde_json::json!({
            "scripts": {"start": "node .", "release": "prisma migrate deploy && node ."},
            "devDependencies": {"prisma": "6"}
        });
        assert_eq!(
            migrate_script(&by_value).map(|(n, _)| n).as_deref(),
            Some("release")
        );

        let library = serde_json::json!({"scripts": {"migrate": "node scripts/migrate.js"}});
        assert!(migrate_script(&library).is_none());
        let tree = RepoTree::from_files(&[("package.json", "{}")]);
        assert!(Node.detect(&tree).unwrap().commands.migrate.is_none());
    }

    #[test]
    fn a_package_without_a_start_script_ranks_low() {
        let tree = RepoTree::from_files(&[
            ("package.json", r#"{"scripts":{"build":"vite build"}}"#),
            ("package-lock.json", ""),
        ]);
        let d = Node.detect(&tree).unwrap();
        assert!(d.confidence <= 40);
        assert!(d.commands.start.is_none());
    }

    #[test]
    fn the_run_environment_puts_the_toolchain_first_on_path() {
        let env = Node.env_for(
            Phase::Run,
            Path::new("/var/lib/ferrum/runtimes/node/22.11.0"),
            Some(20000),
        );
        assert!(env.contains(&(
            "PATH".into(),
            "/var/lib/ferrum/runtimes/node/22.11.0/bin:/usr/local/bin:/usr/bin:/bin".into()
        )));
        assert!(env.contains(&("NODE_ENV".into(), "production".into())));
        let build = Node.env_for(Phase::Build, Path::new("/t"), None);
        assert!(
            !build.iter().any(|(k, _)| k == "NODE_ENV"),
            "NODE_ENV=production at install time drops devDependencies and breaks the build"
        );
    }
}
