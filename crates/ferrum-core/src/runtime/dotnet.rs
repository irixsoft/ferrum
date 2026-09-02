use super::{
    Commands, Detection, Health, Phase, Runtime, RuntimeKind, Source, Target, path_with,
    semver_like,
};
use crate::detect::RepoTree;
use std::path::Path;

pub const INSTALL_SCRIPT: &str = "https://dot.net/v1/dotnet-install.sh";
pub const CHANNELS: [&str; 3] = ["10.0", "9.0", "8.0"];
pub const PACKAGES: [&str; 1] = ["libicu"];
pub const STARTUP_BUDGET_SECS: u32 = 120;

const WEB_SDK: &str = "Microsoft.NET.Sdk.Web";
const BLAZOR_WASM_SDK: &str = "Microsoft.NET.Sdk.BlazorWebAssembly";
const EF_MARKER: &str = "Microsoft.EntityFrameworkCore";

pub struct Dotnet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    pub path: String,
    pub sdk: String,
    pub version: Option<String>,
    pub assembly: String,
    pub uses_ef: bool,
}

impl Project {
    pub fn publish(&self) -> String {
        if self.path.contains('/') {
            format!("dotnet publish {} -c Release -o out", self.path)
        } else {
            "dotnet publish -c Release -o out".to_string()
        }
    }

    pub fn start(&self) -> String {
        format!("dotnet out/{}.dll", self.assembly)
    }
}

pub fn projects(tree: &RepoTree) -> Vec<Project> {
    tree.matching("*.csproj")
        .into_iter()
        .filter_map(|path| {
            let text = tree.read(path)?;
            let stem = path
                .rsplit('/')
                .next()?
                .strip_suffix(".csproj")?
                .to_string();
            Some(Project {
                path: path.to_string(),
                sdk: attribute(text, "Sdk").unwrap_or_default(),
                version: element(text, "TargetFramework")
                    .and_then(|tf| tf.strip_prefix("net").map(str::to_string))
                    .filter(|v| semver_like(v, 2)),
                assembly: element(text, "AssemblyName").unwrap_or(stem),
                uses_ef: text.contains(EF_MARKER),
            })
        })
        .collect()
}

fn attribute(xml: &str, name: &str) -> Option<String> {
    let start = xml.find(&format!("{name}=\""))? + name.len() + 2;
    let end = xml[start..].find('"')? + start;
    Some(xml[start..end].to_string())
}

fn element(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&format!("</{name}>"))? + start;
    Some(xml[start..end].trim().to_string())
}

impl Runtime for Dotnet {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Dotnet
    }

    fn detect(&self, tree: &RepoTree) -> Option<Detection> {
        let projects = projects(tree);
        let project = projects
            .iter()
            .find(|p| p.sdk == WEB_SDK)
            .or_else(|| projects.first())?;

        let (confidence, why) = match project.sdk.as_str() {
            WEB_SDK => (80, format!("{} uses the web SDK", project.path)),
            BLAZOR_WASM_SDK => (
                20,
                format!(
                    "{} is Blazor WebAssembly, which publishes static files",
                    project.path
                ),
            ),
            _ => (60, format!("found {}", project.path)),
        };
        let mut reasons = vec![why];
        if let Some(v) = &project.version {
            reasons.push(format!(".NET {v} from TargetFramework in {}", project.path));
        }
        if project.uses_ef {
            reasons.push(format!("{} references Entity Framework Core", project.path));
        }

        Some(Detection {
            kind: RuntimeKind::Dotnet,
            toolchain: RuntimeKind::Dotnet,
            version: project.version.clone(),
            confidence,
            reasons,
            commands: Commands {
                install: None,
                build: Some(project.publish()),
                start: Some(project.start()),
                migrate: project
                    .uses_ef
                    .then(|| "dotnet ef database update".to_string()),
            },
            output_dir: None,
            health: Health {
                path: "/".into(),
                startup_budget_secs: STARTUP_BUDGET_SECS,
            },
            package_manager: None,
        })
    }

    fn source(&self, version: &str, _target: Target, install_dir: &Path) -> Option<Source> {
        Some(Source::Script {
            url: INSTALL_SCRIPT.into(),
            args: vec![
                "--channel".into(),
                version.into(),
                "--install-dir".into(),
                install_dir.to_string_lossy().into_owned(),
                "--no-path".into(),
            ],
            packages: &PACKAGES,
        })
    }

    fn binary(&self) -> &'static str {
        "dotnet"
    }

    fn valid_version(&self, version: &str) -> bool {
        semver_like(version, 2)
    }

    fn env_for(&self, phase: Phase, toolchain: &Path, port: Option<u16>) -> Vec<(String, String)> {
        let mut env = vec![
            path_with(toolchain),
            (
                "DOTNET_ROOT".into(),
                toolchain.to_string_lossy().into_owned(),
            ),
            ("DOTNET_CLI_TELEMETRY_OPTOUT".into(), "1".into()),
            ("DOTNET_NOLOGO".into(), "1".into()),
        ];
        if phase == Phase::Run {
            env.push(("ASPNETCORE_ENVIRONMENT".into(), "Production".into()));
            env.push(("DOTNET_ENVIRONMENT".into(), "Production".into()));
            if let Some(port) = port {
                env.push(("ASPNETCORE_URLS".into(), format!("http://127.0.0.1:{port}")));
            }
        }
        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_web_project_gets_publish_and_a_120s_startup_budget() {
        let tree = RepoTree::from_files(&[(
            "Api/Api.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk.Web"><PropertyGroup><TargetFramework>net9.0</TargetFramework></PropertyGroup></Project>"#,
        )]);
        let d = Dotnet.detect(&tree).unwrap();
        assert_eq!(d.version.as_deref(), Some("9.0"));
        assert_eq!(
            d.commands.build.as_deref(),
            Some("dotnet publish Api/Api.csproj -c Release -o out")
        );
        assert_eq!(d.commands.start.as_deref(), Some("dotnet out/Api.dll"));
        assert_eq!(d.health.startup_budget_secs, 120);
        assert!(d.commands.migrate.is_none());
        assert_eq!(d.confidence, 80);
    }

    #[test]
    fn a_root_project_publishes_without_naming_itself_and_ef_adds_a_migration() {
        let tree = RepoTree::from_files(&[(
            "Shop.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk.Web"><PropertyGroup><TargetFramework>net10.0</TargetFramework><AssemblyName>ShopWeb</AssemblyName></PropertyGroup><ItemGroup><PackageReference Include="Microsoft.EntityFrameworkCore.Design" /></ItemGroup></Project>"#,
        )]);
        let d = Dotnet.detect(&tree).unwrap();
        assert_eq!(d.version.as_deref(), Some("10.0"));
        assert_eq!(
            d.commands.build.as_deref(),
            Some("dotnet publish -c Release -o out")
        );
        assert_eq!(d.commands.start.as_deref(), Some("dotnet out/ShopWeb.dll"));
        assert_eq!(
            d.commands.migrate.as_deref(),
            Some("dotnet ef database update")
        );
    }

    #[test]
    fn the_web_project_wins_over_a_library_beside_it() {
        let tree = RepoTree::from_files(&[
            ("Core/Core.csproj", r#"<Project Sdk="Microsoft.NET.Sdk">"#),
            ("Web/Web.csproj", r#"<Project Sdk="Microsoft.NET.Sdk.Web">"#),
        ]);
        let d = Dotnet.detect(&tree).unwrap();
        assert_eq!(d.commands.start.as_deref(), Some("dotnet out/Web.dll"));
    }

    #[test]
    fn the_installer_is_driven_by_channel_and_directory() {
        let source = Dotnet
            .source(
                "9.0",
                Target {
                    arch: ferrum_platform::Arch::X86_64,
                    baseline: false,
                },
                Path::new("/var/lib/ferrum/runtimes/dotnet/9.0.partial"),
            )
            .unwrap();
        match source {
            Source::Script {
                url,
                args,
                packages,
            } => {
                assert_eq!(url, INSTALL_SCRIPT);
                assert_eq!(
                    args,
                    vec![
                        "--channel",
                        "9.0",
                        "--install-dir",
                        "/var/lib/ferrum/runtimes/dotnet/9.0.partial",
                        "--no-path"
                    ]
                );
                assert_eq!(packages, &["libicu"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_run_environment_binds_kestrel_to_loopback_on_the_allocated_port() {
        let env = Dotnet.env_for(Phase::Run, Path::new("/t"), Some(20001));
        assert!(env.contains(&("ASPNETCORE_URLS".into(), "http://127.0.0.1:20001".into())));
        assert!(env.contains(&("DOTNET_ROOT".into(), "/t".into())));
        let build = Dotnet.env_for(Phase::Build, Path::new("/t"), None);
        assert!(!build.iter().any(|(k, _)| k == "ASPNETCORE_URLS"));
    }
}
