use crate::apps::NewRoute;
use crate::apps::env::{self, EnvRequirement};
use crate::apps::processes::{NewProcess, WEB};
use crate::detect::RepoTree;
use crate::runtime::{Commands, RuntimeKind};
use ferrum_platform::Platform;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const TOML_NAME: &str = "ferrum.toml";

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
    pub packages: Option<Vec<String>>,
    pub processes: BTreeMap<String, ProcessSpec>,
    pub database: Option<DatabaseSpec>,
    pub redis: Option<RedisSpec>,
    pub env: Option<EnvSpec>,
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
    pub bypass_rls: bool,
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

/// `required = ["A", "B"]` for bare names, `[env.A]` tables when there is something to say; a
/// table alone asks for its key unless it says `optional = true`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EnvSpec {
    pub required: Vec<String>,
    #[serde(flatten)]
    pub keys: BTreeMap<String, EnvKeySpec>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EnvKeySpec {
    pub about: Option<String>,
    pub default: Option<String>,
    pub optional: bool,
}

/// What a repository says about its own shape. Empty `processes` means the file is silent on
/// them; a command, `packages` or `env` left `None` means the file does not state it.
#[derive(Debug, Clone, Serialize)]
pub struct Manifest {
    pub processes: Vec<NewProcess>,
    pub routes: Vec<NewRoute>,
    pub commands: Commands,
    pub packages: Option<Vec<String>>,
    pub database: Option<DatabaseSpec>,
    pub redis: Option<RedisSpec>,
    pub env: Option<Vec<EnvRequirement>>,
}

impl Manifest {
    pub fn states_processes(&self) -> bool {
        !self.processes.is_empty()
    }
}

pub fn parse_toml(text: &str) -> Result<FerrumToml, String> {
    toml::from_str(text).map_err(|e| e.message().to_string())
}

pub fn from_toml(t: &FerrumToml) -> Result<Manifest, String> {
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
    let env = match &t.env {
        Some(spec) => Some(env_requirements(spec, t, &processes)?),
        None => None,
    };
    Ok(Manifest {
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
        env,
    })
}

/// A key Ferrum sets itself cannot be asked for: `PORT`, `HOST`, the port of a process this
/// file names, and its labels. A key in `required` and in a table takes what the table says.
fn env_requirements(
    spec: &EnvSpec,
    t: &FerrumToml,
    processes: &[NewProcess],
) -> Result<Vec<EnvRequirement>, String> {
    let mut ours: Vec<String> = vec!["PORT".into(), "HOST".into()];
    ours.extend(
        processes
            .iter()
            .filter(|p| p.port)
            .map(|p| env::port_var(&p.name)),
    );
    if let Some(db) = &t.database {
        ours.extend(db.url.clone());
        ours.extend(db.roles.values().filter_map(|r| r.url.clone()));
    }
    ours.extend(t.redis.as_ref().and_then(|r| r.url.clone()));

    let mut out: Vec<EnvRequirement> = Vec::new();
    for key in &spec.required {
        if out.iter().any(|r| &r.key == key) {
            continue;
        }
        if spec.keys.get(key).is_some_and(|s| s.optional) {
            return Err(format!(
                "[env] lists {key} as required and marks it optional."
            ));
        }
        out.push(EnvRequirement {
            key: key.clone(),
            ..EnvRequirement::default()
        });
    }
    for (key, table) in &spec.keys {
        let req = EnvRequirement {
            key: key.clone(),
            about: table.about.clone(),
            default: table.default.clone(),
            optional: table.optional,
        };
        match out.iter_mut().find(|r| &r.key == key) {
            Some(listed) => *listed = req,
            None => out.push(req),
        }
    }
    for req in &out {
        env::valid_key(&req.key).map_err(|e| e.to_string())?;
        if ours.contains(&req.key) {
            return Err(format!(
                "[env] names {}, which Ferrum sets itself.",
                req.key
            ));
        }
    }
    Ok(out)
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
    from_toml(&parse_toml(tree.read(TOML_NAME)?).ok()?).ok()
}

/// The manifest of a checked-out release, `Err` when `ferrum.toml` exists but does not hold up.
pub fn read_dir(platform: &dyn Platform, work: &Path) -> anyhow::Result<Option<Manifest>> {
    let Some(text) = platform.read_file(&work.join(TOML_NAME))? else {
        return Ok(None);
    };
    let manifest = parse_toml(&text)
        .and_then(|t| from_toml(&t))
        .map_err(|e| anyhow::anyhow!("{TOML_NAME} could not be read: {e}"))?;
    Ok(Some(manifest))
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
bypass_rls = true

[database.roles.app]
url = "DATABASE_URL"

[redis]
url = "CACHE_URL"

[env]
required = ["SESSION_SECRET", "SMTP_HOST"]

[env.UPLOADS_DIR]
about = "Where uploaded files are kept"
default = "{{shared}}/uploads"
"#,
        )
        .unwrap();
        let m = from_toml(&t).unwrap();
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
        assert!(db.bypass_rls);
        assert_eq!(db.roles["app"].url.as_deref(), Some("DATABASE_URL"));
        assert_eq!(m.redis.unwrap().url.as_deref(), Some("CACHE_URL"));
        assert!(m.packages.is_none(), "a file silent on packages says so");
        let env = m.env.unwrap();
        let keys: Vec<(&str, Option<&str>, Option<&str>)> = env
            .iter()
            .map(|r| (r.key.as_str(), r.about.as_deref(), r.default.as_deref()))
            .collect();
        assert_eq!(
            keys,
            [
                ("SESSION_SECRET", None, None),
                ("SMTP_HOST", None, None),
                (
                    "UPLOADS_DIR",
                    Some("Where uploaded files are kept"),
                    Some("{{shared}}/uploads")
                ),
            ]
        );
    }

    #[test]
    fn an_env_section_refuses_only_the_keys_ferrum_sets_for_this_app() {
        for key in ["PORT", "HOST", "WEB_PORT", "REALTIME_PORT", "DATABASE_URL"] {
            let t = parse_toml(&format!(
                "[env]\nrequired = [\"{key}\"]\n[database]\nurl = \"DATABASE_URL\"\n[processes.web]\nstart = \"a\"\n[processes.realtime]\nstart = \"b\"\nport = true\n[processes.jobs]\nstart = \"c\"\n"
            ))
            .unwrap();
            assert_eq!(
                from_toml(&t).unwrap_err(),
                format!("[env] names {key}, which Ferrum sets itself.")
            );
        }
        let t = parse_toml(
            "[processes.web]\nstart = \"a\"\n[processes.jobs]\nstart = \"c\"\n[env]\nrequired = [\"SMTP_PORT\", \"JOBS_PORT\", \"REALTIME_PORT\"]\n",
        )
        .unwrap();
        assert_eq!(
            from_toml(&t).unwrap().env.unwrap().len(),
            3,
            "a name ending in _PORT is the app's own unless a port process carries it"
        );
        let bad = parse_toml("[env]\nrequired = [\"1bad\"]\n").unwrap();
        assert!(
            from_toml(&bad)
                .unwrap_err()
                .contains("not a valid variable name")
        );
        let none = from_toml(&parse_toml("[env]\n").unwrap()).unwrap();
        assert_eq!(
            none.env,
            Some(Vec::new()),
            "an empty section asks for nothing"
        );
        assert!(from_toml(&parse_toml("").unwrap()).unwrap().env.is_none());
    }

    #[test]
    fn an_env_key_is_required_unless_its_table_says_optional() {
        let t = parse_toml(
            r#"
[env]
required = ["SESSION_SECRET", "ADMIN_EMAIL", "SESSION_SECRET"]

[env.ADMIN_EMAIL]
about = "The first admin's login"

[env.SMTP_PORT]
default = "587"

[env.SMTP_USER]
about = "Leave empty for a relay without a login"
optional = true
"#,
        )
        .unwrap();
        let env = from_toml(&t).unwrap().env.unwrap();
        let keys: Vec<(&str, Option<&str>, Option<&str>, bool)> = env
            .iter()
            .map(|r| {
                (
                    r.key.as_str(),
                    r.about.as_deref(),
                    r.default.as_deref(),
                    r.optional,
                )
            })
            .collect();
        assert_eq!(
            keys,
            [
                ("SESSION_SECRET", None, None, false),
                ("ADMIN_EMAIL", Some("The first admin's login"), None, false),
                ("SMTP_PORT", None, Some("587"), false),
                (
                    "SMTP_USER",
                    Some("Leave empty for a relay without a login"),
                    None,
                    true
                ),
            ]
        );
        let both = parse_toml("[env]\nrequired = [\"A\"]\n[env.A]\noptional = true\n").unwrap();
        assert_eq!(
            from_toml(&both).unwrap_err(),
            "[env] lists A as required and marks it optional."
        );
    }

    #[test]
    fn a_plain_start_is_one_web_process_and_an_output_dir_is_one_folder() {
        let t = parse_toml(
            "runtime = \"bun\"\nstart = \"bun run src/main.ts\"\nhealth_path = \"/up\"\n",
        )
        .unwrap();
        let m = from_toml(&t).unwrap();
        assert_eq!(
            m.processes,
            vec![NewProcess::web("bun run src/main.ts", Some("/up"))]
        );
        assert_eq!(m.routes.len(), 1);
        assert_eq!(m.routes[0].process, "web");

        let t = parse_toml("build = \"bun run build\"\noutput_dir = \"dist\"\n").unwrap();
        let m = from_toml(&t).unwrap();
        assert_eq!(m.processes, vec![NewProcess::folder("web", "dist")]);
        assert_eq!(m.routes[0].path, "/");

        let silent = from_toml(&parse_toml("packages = [\"ffmpeg\"]\n").unwrap()).unwrap();
        assert!(!silent.states_processes());
        assert!(silent.routes.is_empty());
        assert_eq!(
            silent.packages.as_deref(),
            Some(["ffmpeg".to_string()].as_slice())
        );
        let empty = from_toml(&parse_toml("packages = []\n").unwrap()).unwrap();
        assert_eq!(
            empty.packages,
            Some(Vec::new()),
            "an empty list drops every package"
        );
    }

    #[test]
    fn a_process_that_is_not_web_has_a_port_only_when_it_says_so_or_has_a_health_path() {
        let t = parse_toml(
            "[processes.api]\nstart = \"a\"\nhealth = \"/up\"\n[processes.site]\nstart = \"b\"\n[processes.admin]\nstart = \"c\"\nport = true\n",
        )
        .unwrap();
        let m = from_toml(&t).unwrap();
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
    fn a_tree_is_read_from_ferrum_toml_alone() {
        let tree =
            RepoTree::from_files(&[("ferrum.toml", "start = \"a\"\n"), ("Procfile", "web: b\n")]);
        assert_eq!(
            read(&tree).unwrap().processes[0].start.as_deref(),
            Some("a")
        );
        let procfile_only = RepoTree::from_files(&[("Procfile", "web: b\n")]);
        assert!(read(&procfile_only).is_none(), "a Procfile is not read");
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
        assert!(read_dir(&p, work).unwrap().is_none());
        p.write_file(
            &work.join("ferrum.toml"),
            "[processes.web]\nstart = \"x\"\n",
            0o644,
        )
        .unwrap();
        assert_eq!(
            read_dir(&p, work).unwrap().unwrap().processes[0].name,
            "web"
        );
        p.write_file(
            &work.join("ferrum.toml"),
            "[env]\nrequired = [\"PORT\"]\n",
            0o644,
        )
        .unwrap();
        let e = read_dir(&p, work).unwrap_err().to_string();
        assert!(
            e.starts_with("ferrum.toml could not be read: [env] names PORT"),
            "{e}"
        );
        p.write_file(&work.join("ferrum.toml"), "= broken", 0o644)
            .unwrap();
        let err = read_dir(&p, work).unwrap_err().to_string();
        assert!(err.starts_with("ferrum.toml could not be read:"), "{err}");
    }
}
