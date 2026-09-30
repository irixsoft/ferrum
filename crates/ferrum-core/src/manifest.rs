use crate::apps::NewRoute;
use crate::apps::processes::{self, NewProcess, WEB};
use crate::detect::RepoTree;
use crate::runtime::{Commands, RuntimeKind};
use ferrum_platform::Platform;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const TOML_NAME: &str = "ferrum.toml";
pub const PROCFILE_NAME: &str = "Procfile";
const RELEASE: &str = "release";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FerrumToml {
    pub runtime: Option<RuntimeKind>,
    pub version: Option<String>,
    pub install: Option<String>,
    pub build: Option<String>,
    pub start: Option<String>,
    pub migrate: Option<String>,
    pub output_dir: Option<String>,
    pub health_path: Option<String>,
    pub packages: Vec<String>,
    pub processes: BTreeMap<String, ProcessSpec>,
    pub database: Option<DatabaseSpec>,
    pub redis: Option<RedisSpec>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessSpec {
    pub start: Option<String>,
    pub dir: String,
    pub port: Option<bool>,
    pub health: Option<String>,
    #[serde(rename = "static")]
    pub static_dir: Option<String>,
    pub path: Option<String>,
    pub websocket: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DatabaseSpec {
    pub url: Option<String>,
    pub roles: BTreeMap<String, RoleSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RoleSpec {
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RedisSpec {
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    FerrumToml,
    Procfile,
}

impl Source {
    pub fn file_name(self) -> &'static str {
        match self {
            Source::FerrumToml => TOML_NAME,
            Source::Procfile => PROCFILE_NAME,
        }
    }
}

/// What a repository says about its own shape. Empty `processes` means the file is silent on
/// them; a command left `None` means the file does not state it.
#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    pub source: Source,
    pub processes: Vec<NewProcess>,
    pub routes: Vec<NewRoute>,
    pub commands: Commands,
    pub packages: Vec<String>,
    pub database: Option<DatabaseSpec>,
    pub redis: Option<RedisSpec>,
}

impl Manifest {
    pub fn states_processes(&self) -> bool {
        !self.processes.is_empty()
    }
}

pub fn parse_toml(text: &str) -> Result<FerrumToml, String> {
    toml::from_str(text).map_err(|e| e.message().to_string())
}

pub fn parse_procfile(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (name, command) = l.split_once(':')?;
            let (name, command) = (name.trim(), command.trim());
            (processes::valid_name(name) || name == RELEASE)
                .then(|| (name.to_string(), command.to_string()))
                .filter(|(_, c)| !c.is_empty())
        })
        .collect()
}

pub fn from_toml(t: &FerrumToml) -> Manifest {
    let mut processes = Vec::new();
    let mut routes = Vec::new();
    if !t.processes.is_empty() {
        for (name, spec) in &t.processes {
            let folder = spec.static_dir.is_some();
            let port = !folder && spec.port.unwrap_or(name == WEB || spec.health.is_some());
            processes.push(NewProcess {
                name: name.clone(),
                start: spec.start.clone(),
                dir: spec.dir.clone(),
                port,
                health: spec.health.clone(),
                static_dir: spec.static_dir.clone(),
                memory_mb: None,
            });
            if let Some(path) = &spec.path {
                routes.push(NewRoute {
                    path: path.clone(),
                    process: name.clone(),
                    websocket: spec.websocket,
                });
            }
        }
    } else if let Some(start) = &t.start {
        processes.push(NewProcess::web(start, t.health_path.as_deref()));
    } else if let Some(dir) = &t.output_dir {
        processes.push(NewProcess::folder(WEB, dir));
    }
    Manifest {
        source: Source::FerrumToml,
        routes: routes_for(&processes, routes),
        processes,
        commands: Commands {
            install: t.install.clone(),
            build: t.build.clone(),
            migrate: t.migrate.clone(),
        },
        packages: t.packages.clone(),
        database: t.database.clone(),
        redis: t.redis.clone(),
    }
}

pub fn from_procfile(entries: &[(String, String)]) -> Manifest {
    let mut processes = Vec::new();
    let mut migrate = None;
    for (name, command) in entries {
        if name == RELEASE {
            migrate = Some(command.clone());
        } else if name == WEB {
            processes.push(NewProcess::web(command, None));
        } else {
            processes.push(NewProcess::worker(name, command));
        }
    }
    Manifest {
        source: Source::Procfile,
        routes: routes_for(&processes, Vec::new()),
        processes,
        commands: Commands {
            install: None,
            build: None,
            migrate,
        },
        packages: Vec::new(),
        database: None,
        redis: None,
    }
}

/// The paths a repo names, plus `/` on the first process that can answer it when nobody
/// claimed it: the first port process without a path of its own, else the first port process,
/// else the first folder.
fn routes_for(processes: &[NewProcess], mut routes: Vec<NewRoute>) -> Vec<NewRoute> {
    if processes.is_empty() || routes.iter().any(|r| r.path == "/") {
        return routes;
    }
    let claimed: Vec<&str> = routes.iter().map(|r| r.process.as_str()).collect();
    let root = processes
        .iter()
        .find(|p| p.port && !claimed.contains(&p.name.as_str()))
        .or_else(|| processes.iter().find(|p| p.port))
        .or_else(|| processes.iter().find(|p| p.is_folder()));
    if let Some(p) = root {
        routes.insert(
            0,
            NewRoute {
                path: "/".into(),
                process: p.name.clone(),
                websocket: false,
            },
        );
    }
    routes
}

pub fn read(tree: &RepoTree) -> Option<Manifest> {
    if let Some(text) = tree.read(TOML_NAME) {
        return parse_toml(text).ok().map(|t| from_toml(&t));
    }
    let entries = parse_procfile(tree.read(PROCFILE_NAME)?);
    (!entries.is_empty()).then(|| from_procfile(&entries))
}

/// The manifest of a checked-out release, `Err` when `ferrum.toml` exists but does not parse.
pub fn read_dir(platform: &dyn Platform, work: &Path) -> anyhow::Result<Option<Manifest>> {
    if let Some(text) = platform.read_file(&work.join(TOML_NAME))? {
        let parsed =
            parse_toml(&text).map_err(|e| anyhow::anyhow!("{TOML_NAME} could not be read: {e}"))?;
        return Ok(Some(from_toml(&parsed)));
    }
    let Some(text) = platform.read_file(&work.join(PROCFILE_NAME))? else {
        return Ok(None);
    };
    let entries = parse_procfile(&text);
    Ok((!entries.is_empty()).then(|| from_procfile(&entries)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_processes_table_names_each_process_with_its_folder_port_health_and_path() {
        let t = parse_toml(
            r#"
build = "bun run --filter web build"
migrate = "bun run db:migrate"

[processes.web]
start = "bun run start"
dir = "apps/web"
port = true
health = "/api/healthz"

[processes.realtime]
start = "bun run start"
dir = "apps/realtime"
port = true
path = "/live"
websocket = true

[processes.jobs]
start = "bun run start"
dir = "apps/jobs"

[processes.admin]
static = "apps/admin/dist"
path = "/admin"

[database]
url = "DATABASE_ADMIN_URL"

[database.roles.app]
url = "DATABASE_URL"

[redis]
url = "CACHE_URL"
"#,
        )
        .unwrap();
        let m = from_toml(&t);
        assert_eq!(m.source, Source::FerrumToml);
        let names: Vec<&str> = m.processes.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["admin", "jobs", "realtime", "web"]);
        let web = m.processes.iter().find(|p| p.name == "web").unwrap();
        assert!(web.port);
        assert_eq!(web.dir, "apps/web");
        assert_eq!(web.health.as_deref(), Some("/api/healthz"));
        let jobs = m.processes.iter().find(|p| p.name == "jobs").unwrap();
        assert!(!jobs.port);
        let admin = m.processes.iter().find(|p| p.name == "admin").unwrap();
        assert!(admin.is_folder());
        assert!(!admin.port);
        let paths: Vec<(&str, &str, bool)> = m
            .routes
            .iter()
            .map(|r| (r.path.as_str(), r.process.as_str(), r.websocket))
            .collect();
        assert_eq!(
            paths,
            [
                ("/", "web", false),
                ("/admin", "admin", false),
                ("/live", "realtime", true)
            ]
        );
        assert_eq!(
            m.commands.build.as_deref(),
            Some("bun run --filter web build")
        );
        assert_eq!(m.commands.migrate.as_deref(), Some("bun run db:migrate"));
        assert!(m.commands.install.is_none());
        let db = m.database.unwrap();
        assert_eq!(db.url.as_deref(), Some("DATABASE_ADMIN_URL"));
        assert_eq!(db.roles["app"].url.as_deref(), Some("DATABASE_URL"));
        assert_eq!(m.redis.unwrap().url.as_deref(), Some("CACHE_URL"));
    }

    #[test]
    fn a_plain_start_is_one_web_process_and_an_output_dir_is_one_folder() {
        let t = parse_toml(
            "runtime = \"bun\"\nstart = \"bun run src/main.ts\"\nhealth_path = \"/up\"\n",
        )
        .unwrap();
        let m = from_toml(&t);
        assert_eq!(
            m.processes,
            vec![NewProcess::web("bun run src/main.ts", Some("/up"))]
        );
        assert_eq!(m.routes.len(), 1);
        assert_eq!(m.routes[0].process, "web");

        let t = parse_toml("build = \"bun run build\"\noutput_dir = \"dist\"\n").unwrap();
        let m = from_toml(&t);
        assert_eq!(m.processes, vec![NewProcess::folder("web", "dist")]);
        assert_eq!(m.routes[0].path, "/");

        let silent = from_toml(&parse_toml("packages = [\"ffmpeg\"]\n").unwrap());
        assert!(!silent.states_processes());
        assert!(silent.routes.is_empty());
        assert_eq!(silent.packages, ["ffmpeg"]);
    }

    #[test]
    fn a_process_that_is_not_web_has_a_port_only_when_it_says_so_or_has_a_health_path() {
        let t = parse_toml(
            "[processes.api]\nstart = \"a\"\nhealth = \"/up\"\n[processes.site]\nstart = \"b\"\n[processes.admin]\nstart = \"c\"\nport = true\n",
        )
        .unwrap();
        let m = from_toml(&t);
        let port = |n: &str| m.processes.iter().find(|p| p.name == n).unwrap().port;
        assert!(port("api"));
        assert!(!port("site"));
        assert!(port("admin"));
        assert_eq!(
            m.routes[0].process, "admin",
            "the first port process takes /"
        );
    }

    #[test]
    fn a_procfile_gives_web_a_port_workers_none_and_release_becomes_migrate() {
        let entries = parse_procfile(
            "# comment\nweb: bun run start\nworker: bun run worker\n\nrelease: bun run migrate\nBad Name: x\nempty:\n",
        );
        assert_eq!(entries.len(), 3);
        let m = from_procfile(&entries);
        assert_eq!(m.source, Source::Procfile);
        assert_eq!(
            m.processes,
            vec![
                NewProcess::web("bun run start", None),
                NewProcess::worker("worker", "bun run worker")
            ]
        );
        assert_eq!(m.commands.migrate.as_deref(), Some("bun run migrate"));
        assert_eq!(m.routes[0].process, "web");
    }

    #[test]
    fn a_tree_prefers_ferrum_toml_and_falls_back_to_a_procfile() {
        let both =
            RepoTree::from_files(&[("ferrum.toml", "start = \"a\"\n"), ("Procfile", "web: b\n")]);
        assert_eq!(
            read(&both).unwrap().processes[0].start.as_deref(),
            Some("a")
        );
        let only = RepoTree::from_files(&[("Procfile", "web: b\n")]);
        assert_eq!(read(&only).unwrap().source, Source::Procfile);
        assert!(read(&RepoTree::from_files(&[("package.json", "{}")])).is_none());
        assert!(read(&RepoTree::from_files(&[("ferrum.toml", "= broken")])).is_none());
    }

    #[test]
    fn a_release_directory_is_read_the_same_way_and_a_broken_toml_is_an_error() {
        let p = ferrum_platform::FakePlatform::new();
        let work = Path::new("/var/lib/ferrum/apps/x/releases/r1");
        assert!(read_dir(&p, work).unwrap().is_none());
        p.write_file(&work.join("Procfile"), "web: bun run start\n", 0o644)
            .unwrap();
        assert_eq!(
            read_dir(&p, work).unwrap().unwrap().source,
            Source::Procfile
        );
        p.write_file(
            &work.join("ferrum.toml"),
            "[processes.web]\nstart = \"x\"\n",
            0o644,
        )
        .unwrap();
        assert_eq!(
            read_dir(&p, work).unwrap().unwrap().source,
            Source::FerrumToml
        );
        p.write_file(&work.join("ferrum.toml"), "= broken", 0o644)
            .unwrap();
        let err = read_dir(&p, work).unwrap_err().to_string();
        assert!(err.starts_with("ferrum.toml could not be read:"), "{err}");
    }
}
