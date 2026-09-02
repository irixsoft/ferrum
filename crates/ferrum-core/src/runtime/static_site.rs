use super::{
    Commands, Detection, Health, PackageManager, Phase, Runtime, RuntimeKind, Source, Target,
    dotnet, node,
};
use crate::detect::RepoTree;
use std::path::Path;

pub struct Static;

const BLAZOR_WASM_SDK: &str = "Microsoft.NET.Sdk.BlazorWebAssembly";
const SSR_ASTRO_ADAPTERS: [&str; 3] = ["@astrojs/node", "@astrojs/vercel", "@astrojs/cloudflare"];

struct Site {
    output_dir: &'static str,
    confidence: u8,
    reason: String,
}

fn javascript_site(tree: &RepoTree, package: &serde_json::Value) -> Option<Site> {
    let deps = |name: &str| {
        package["dependencies"][name].is_string() || package["devDependencies"][name].is_string()
    };
    if let Some(config) = tree.matching("vite.config.*").first() {
        return Some(Site {
            output_dir: "dist",
            confidence: 90,
            reason: format!("found {config}"),
        });
    }
    if let Some(config) = tree.matching("astro.config.*").first() {
        if SSR_ASTRO_ADAPTERS.iter().any(|a| deps(a)) {
            return None;
        }
        return Some(Site {
            output_dir: "dist",
            confidence: 90,
            reason: format!("found {config} with no server adapter"),
        });
    }
    if deps("vitepress") {
        return Some(Site {
            output_dir: ".vitepress/dist",
            confidence: 85,
            reason: "found vitepress in package.json".into(),
        });
    }
    if deps("react-scripts") {
        return Some(Site {
            output_dir: "build",
            confidence: 85,
            reason: "found react-scripts in package.json".into(),
        });
    }
    let (has_build, has_start) = node::scripts(package);
    if has_build && !has_start && node::framework(tree).is_none() {
        return Some(Site {
            output_dir: "dist",
            confidence: 50,
            reason: "a build script and no start script".into(),
        });
    }
    None
}

impl Runtime for Static {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Static
    }

    fn detect(&self, tree: &RepoTree) -> Option<Detection> {
        if let Some(project) = dotnet::projects(tree)
            .into_iter()
            .find(|p| p.sdk == BLAZOR_WASM_SDK)
        {
            return Some(Detection {
                kind: RuntimeKind::Static,
                toolchain: RuntimeKind::Dotnet,
                version: project.version.clone(),
                confidence: 80,
                reasons: vec![format!("{} uses the Blazor WebAssembly SDK", project.path)],
                commands: Commands {
                    install: None,
                    build: Some(project.publish()),
                    start: None,
                    migrate: None,
                },
                output_dir: Some("out/wwwroot".into()),
                health: Health::default(),
                package_manager: None,
            });
        }

        let package = tree.json("package.json")?;
        let (has_start, start) = {
            let (_, has_start) = node::scripts(&package);
            (
                has_start,
                package["scripts"]["start"].as_str().unwrap_or(""),
            )
        };
        if has_start && !start.starts_with("vite preview") && !start.starts_with("serve ") {
            return None;
        }
        let site = javascript_site(tree, &package)?;

        let mut reasons = vec!["found package.json".to_string(), site.reason];
        let (pm, locked) = match node::package_manager(tree) {
            Some((pm, lock)) => {
                reasons.push(format!("found {lock}"));
                (pm, true)
            }
            None => (PackageManager::Npm, false),
        };
        let version = node::wanted_version(tree, &package).map(|(v, why)| {
            reasons.push(why);
            v
        });

        Some(Detection {
            kind: RuntimeKind::Static,
            toolchain: if pm == PackageManager::Bun {
                RuntimeKind::Bun
            } else {
                RuntimeKind::Node
            },
            version,
            confidence: site.confidence,
            reasons,
            commands: Commands {
                install: Some(pm.install(locked).to_string()),
                build: Some(pm.run("build")),
                start: None,
                migrate: None,
            },
            output_dir: Some(site.output_dir.into()),
            health: Health::default(),
            package_manager: Some(pm),
        })
    }

    fn source(&self, _version: &str, _target: Target, _install_dir: &Path) -> Option<Source> {
        None
    }

    fn binary(&self) -> &'static str {
        ""
    }

    fn valid_version(&self, _version: &str) -> bool {
        false
    }

    fn env_for(
        &self,
        _phase: Phase,
        _toolchain: &Path,
        _port: Option<u16>,
    ) -> Vec<(String, String)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vite_without_a_start_script_is_static_with_dist() {
        let tree = RepoTree::from_files(&[
            ("package.json", r#"{"scripts":{"build":"vite build"}}"#),
            ("vite.config.ts", ""),
        ]);
        let d = Static.detect(&tree).unwrap();
        assert_eq!(d.output_dir.as_deref(), Some("dist"));
        assert!(d.commands.start.is_none(), "a static site has no process");
        assert_eq!(d.commands.build.as_deref(), Some("npm run build"));
        assert_eq!(d.toolchain, RuntimeKind::Node);
    }

    #[test]
    fn a_bun_lockfile_builds_the_site_with_bun() {
        let tree = RepoTree::from_files(&[
            ("package.json", r#"{"scripts":{"build":"vite build"}}"#),
            ("vite.config.ts", ""),
            ("bun.lock", ""),
        ]);
        let d = Static.detect(&tree).unwrap();
        assert_eq!(d.toolchain, RuntimeKind::Bun);
        assert_eq!(d.commands.build.as_deref(), Some("bun run build"));
    }

    #[test]
    fn a_server_start_script_is_not_a_static_site() {
        let tree = RepoTree::from_files(&[
            (
                "package.json",
                r#"{"scripts":{"build":"vite build","start":"node server.js"}}"#,
            ),
            ("vite.config.ts", ""),
        ]);
        assert!(Static.detect(&tree).is_none());
    }

    #[test]
    fn astro_with_a_node_adapter_is_a_server() {
        let tree = RepoTree::from_files(&[
            (
                "package.json",
                r#"{"scripts":{"build":"astro build"},"dependencies":{"@astrojs/node":"1"}}"#,
            ),
            ("astro.config.mjs", ""),
        ]);
        assert!(Static.detect(&tree).is_none());
    }

    #[test]
    fn blazor_wasm_standalone_is_static_and_blazor_server_is_not() {
        let wasm = RepoTree::from_files(&[(
            "App.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk.BlazorWebAssembly"><PropertyGroup><TargetFramework>net9.0</TargetFramework></PropertyGroup></Project>"#,
        )]);
        let site = Static.detect(&wasm).unwrap();
        assert_eq!(site.toolchain, RuntimeKind::Dotnet);
        assert_eq!(site.version.as_deref(), Some("9.0"));
        assert_eq!(site.output_dir.as_deref(), Some("out/wwwroot"));
        assert!(
            dotnet::Dotnet
                .detect(&wasm)
                .map(|d| d.confidence)
                .unwrap_or(0)
                < site.confidence
        );

        let server =
            RepoTree::from_files(&[("App.csproj", r#"<Project Sdk="Microsoft.NET.Sdk.Web">"#)]);
        assert!(Static.detect(&server).is_none());
        assert!(dotnet::Dotnet.detect(&server).is_some());
    }
}
