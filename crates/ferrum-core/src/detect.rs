use crate::github::Api;
use crate::manifest::{self, Manifest};
use crate::runtime::{self, Detection, node, static_site};
use crate::state::State;
use serde::Serialize;
use std::collections::HashMap;

pub const TOO_LARGE: &str = "The repository tree is too large to inspect. Set the root directory to the application's folder, or fill in the settings by hand.";

const WANTED: [&str; 8] = [
    "package.json",
    ".nvmrc",
    ".node-version",
    ".bun-version",
    "global.json",
    "ferrum.toml",
    "README.md",
    runtime::dotnet::TOOL_MANIFEST,
];
const WANTED_GLOBS: [&str; 1] = ["*.csproj"];
const MAX_PROJECT_FILES: usize = 10;

const POSTGRES_CLIENTS: [&str; 7] = [
    "pg",
    "postgres",
    "pg-promise",
    "@vercel/postgres",
    "@neondatabase/serverless",
    "@payloadcms/db-postgres",
    "drizzle-orm",
];
const REDIS_CLIENTS: [&str; 4] = ["ioredis", "redis", "bullmq", "connect-redis"];

#[derive(Debug, Clone, Default)]
pub struct RepoTree {
    paths: Vec<String>,
    files: HashMap<String, String>,
}

impl RepoTree {
    pub fn from_files(files: &[(&str, &str)]) -> Self {
        Self {
            paths: files.iter().map(|(p, _)| p.to_string()).collect(),
            files: files
                .iter()
                .map(|(p, c)| (p.to_string(), c.to_string()))
                .collect(),
        }
    }

    pub fn has(&self, path: &str) -> bool {
        self.paths.iter().any(|p| p == path)
    }

    pub fn any(&self, glob: &str) -> bool {
        !self.matching(glob).is_empty()
    }

    pub fn matching(&self, glob: &str) -> Vec<&str> {
        self.paths
            .iter()
            .map(String::as_str)
            .filter(|p| glob_matches(glob, p))
            .collect()
    }

    pub fn read(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    pub fn json(&self, path: &str) -> Option<serde_json::Value> {
        serde_json::from_str(self.read(path)?).ok()
    }

    fn wanted(&self) -> Vec<String> {
        let mut names: Vec<String> = WANTED
            .iter()
            .filter(|n| self.has(n))
            .map(|n| n.to_string())
            .collect();
        for glob in WANTED_GLOBS {
            names.extend(
                self.matching(glob)
                    .into_iter()
                    .take(MAX_PROJECT_FILES)
                    .map(str::to_string),
            );
        }
        names.retain(|n| n != "README.md");
        names
    }
}

/// `*.csproj` matches at any depth; a pattern with a slash matches the whole path.
fn glob_matches(glob: &str, path: &str) -> bool {
    let subject = if glob.contains('/') {
        path
    } else {
        path.rsplit('/').next().unwrap_or(path)
    };
    let mut parts = glob.split('*');
    let first = parts.next().unwrap_or("");
    if !subject.starts_with(first) {
        return false;
    }
    let mut rest = &subject[first.len()..];
    let remaining: Vec<&str> = parts.collect();
    for (i, part) in remaining.iter().enumerate() {
        let last = i == remaining.len() - 1;
        if last {
            return rest.ends_with(part);
        }
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.is_empty()
}

/// Why the repository looks like it needs a database, if it does.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Wants {
    pub postgres: Option<String>,
    pub redis: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Detected {
    pub candidates: Vec<Detection>,
    pub manifest: Option<Manifest>,
    pub wants: Wants,
}

#[derive(Debug, thiserror::Error)]
pub enum DetectError {
    #[error("{TOO_LARGE}")]
    TooLarge,
    #[error("{0}")]
    NoSuchRef(String),
}

pub async fn inspect(
    api: &Api,
    state: &State,
    full_name: &str,
    git_ref: &str,
    root: &str,
) -> anyhow::Result<Detected> {
    let listing = api.tree(state, full_name, git_ref).await?;
    if listing.truncated {
        return Err(DetectError::TooLarge.into());
    }

    let prefix = root.trim_matches('/');
    let mut tree = RepoTree {
        paths: listing
            .paths
            .iter()
            .filter_map(|p| under(p, prefix))
            .map(str::to_string)
            .collect(),
        files: HashMap::new(),
    };

    for name in tree.wanted() {
        let full = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if let Some(contents) = api.file(state, full_name, git_ref, &full).await? {
            tree.files.insert(name, contents);
        }
    }

    Ok(detect(&tree))
}

pub fn detect(tree: &RepoTree) -> Detected {
    let mut candidates: Vec<Detection> = runtime::all()
        .iter()
        .filter_map(|r| r.detect(tree))
        .chain(static_site::detect(tree))
        .collect();
    candidates.sort_by_key(|c| std::cmp::Reverse(c.confidence));

    Detected {
        candidates,
        manifest: manifest::read(tree),
        wants: wants(tree),
    }
}

pub fn wants(tree: &RepoTree) -> Wants {
    let package = tree.json("package.json");
    let from_package = |clients: &[&str]| {
        package
            .as_ref()
            .and_then(|p| node::depends_on(p, clients))
            .map(|dep| format!("{dep} in dependencies"))
    };
    let from_csproj = || {
        tree.matching("*.csproj")
            .into_iter()
            .find(|p| tree.read(p).is_some_and(|c| c.contains("Npgsql")))
            .map(|p| format!("Npgsql in {p}"))
    };
    Wants {
        postgres: from_package(&POSTGRES_CLIENTS).or_else(from_csproj),
        redis: from_package(&REDIS_CLIENTS),
    }
}

fn under<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    if prefix.is_empty() {
        return Some(path);
    }
    path.strip_prefix(prefix)?.strip_prefix('/')
}

/// `^[a-z0-9][a-z0-9+._-]*$` — a package name reaches `apt-get` as one argv entry.
pub fn valid_package(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "+._-".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_names_are_argv_safe_or_refused() {
        for good in ["ffmpeg", "libvips42", "g++", "libssl-dev", "python3.12"] {
            assert!(valid_package(good), "{good}");
        }
        for bad in [
            "",
            "-x",
            "Ffmpeg",
            "libvips; rm -rf /",
            "a b",
            "../x",
            "$(id)",
        ] {
            assert!(!valid_package(bad), "{bad}");
        }
    }

    #[test]
    fn globs_match_by_name_at_any_depth() {
        let tree = RepoTree::from_files(&[("Api/Api.csproj", ""), ("README.md", "")]);
        assert!(tree.any("*.csproj"));
        assert_eq!(tree.matching("*.csproj"), vec!["Api/Api.csproj"]);
        assert!(!tree.any("*.sln"));
        assert!(tree.any("Api/*.csproj"));
        assert!(!tree.any("Web/*.csproj"));
        assert!(!tree.any("next.config.*"));
    }

    #[test]
    fn only_files_a_runtime_reads_are_wanted() {
        let tree = RepoTree::from_files(&[
            ("package.json", ""),
            ("next.config.js", ""),
            ("README.md", ""),
            ("src/index.ts", ""),
            (".env.example", ""),
            ("src/env.ts", ""),
            ("Aptfile", ""),
            ("Procfile", ""),
            ("ferrum.toml", ""),
        ]);
        assert_eq!(
            tree.wanted(),
            vec!["package.json", "ferrum.toml"],
            "no example files, no Aptfile, no Procfile: ferrum.toml says it all"
        );
    }

    #[test]
    fn a_database_is_wanted_from_the_dependencies_or_the_csproj() {
        let deps = RepoTree::from_files(&[(
            "package.json",
            r#"{"dependencies":{"drizzle-orm":"1","ioredis":"5"}}"#,
        )]);
        assert_eq!(
            wants(&deps),
            Wants {
                postgres: Some("drizzle-orm in dependencies".into()),
                redis: Some("ioredis in dependencies".into()),
            }
        );
        let dotnet = RepoTree::from_files(&[(
            "Api/Api.csproj",
            r#"<PackageReference Include="Npgsql.EntityFrameworkCore.PostgreSQL" />"#,
        )]);
        assert_eq!(
            wants(&dotnet).postgres.as_deref(),
            Some("Npgsql in Api/Api.csproj")
        );
        assert_eq!(wants(&RepoTree::default()), Wants::default());
    }

    #[test]
    fn the_root_directory_scopes_the_tree() {
        assert_eq!(
            under("apps/web/package.json", "apps/web"),
            Some("package.json")
        );
        assert_eq!(under("apps/website/x", "apps/web"), None);
        assert_eq!(under("package.json", ""), Some("package.json"));
    }

    #[test]
    fn a_manifest_in_the_tree_is_read_without_needing_every_key() {
        let tree = RepoTree::from_files(&[
            (
                "ferrum.toml",
                "runtime = \"bun\"\nstart = \"bun run src/main.ts\"\n",
            ),
            ("package.json", "{}"),
        ]);
        let found = detect(&tree);
        let manifest = found.manifest.unwrap();
        assert_eq!(
            manifest.processes[0].start.as_deref(),
            Some("bun run src/main.ts")
        );
        assert!(manifest.commands.build.is_none());
    }

    #[test]
    fn candidates_come_best_first() {
        let tree = RepoTree::from_files(&[
            ("package.json", r#"{"scripts":{"build":"vite build"}}"#),
            ("vite.config.ts", ""),
            ("package-lock.json", ""),
        ]);
        let found = detect(&tree);
        assert_eq!(found.candidates[0].output_dir(), Some("dist"));
        assert!(
            found
                .candidates
                .windows(2)
                .all(|w| w[0].confidence >= w[1].confidence)
        );
    }
}
