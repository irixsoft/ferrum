use super::{ArchiveFormat, Mirrors, Runtime, RuntimeKind, Source, Target, by_kind};
use crate::apps::App;
use crate::state::State;
use crate::time;
use anyhow::{Context, bail};
use ferrum_platform::Platform;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const RUNTIMES_DIR: &str = "/var/lib/ferrum/runtimes";
const SCRIPT_NAME: &str = "install.sh";

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self::at(RUNTIMES_DIR)
    }
}

impl Store {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn dir(&self, kind: RuntimeKind, version: &str) -> PathBuf {
        self.root.join(kind.as_str()).join(version)
    }

    fn partial(&self, kind: RuntimeKind, version: &str) -> PathBuf {
        self.root
            .join(kind.as_str())
            .join(format!("{version}.partial"))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Toolchain {
    pub kind: RuntimeKind,
    pub version: String,
    pub path: String,
    pub size_bytes: i64,
    pub installed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Progress {
    Downloading { received: u64, total: Option<u64> },
    Extracting,
    Installing,
    Ready,
    Failed { error: String },
}

pub async fn installed(state: &State) -> anyhow::Result<Vec<Toolchain>> {
    let rows = sqlx::query!(
        r#"SELECT kind AS "kind!: RuntimeKind", version AS "version!", path AS "path!",
                  size_bytes AS "size_bytes!", installed_at AS "installed_at!"
           FROM toolchains ORDER BY kind, version"#
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Toolchain {
            kind: r.kind,
            version: r.version,
            path: r.path,
            size_bytes: r.size_bytes,
            installed_at: time::utc(r.installed_at),
        })
        .collect())
}

pub async fn find(
    state: &State,
    kind: RuntimeKind,
    version: &str,
) -> anyhow::Result<Option<Toolchain>> {
    Ok(installed(state)
        .await?
        .into_iter()
        .find(|t| t.kind == kind && t.version == version))
}

/// The newest installed toolchain of the other tool an app's commands start with: Bun beside
/// Node when a command starts with `bun`, and the reverse. Never installs one.
pub async fn extra_for(state: &State, store: &Store, app: &App) -> anyhow::Result<Option<PathBuf>> {
    let wanted = [
        &app.commands.install,
        &app.commands.build,
        &app.commands.start,
    ]
    .into_iter()
    .flatten()
    .filter_map(|c| match c.split_whitespace().next() {
        Some("bun" | "bunx") => Some(RuntimeKind::Bun),
        Some("npm" | "npx" | "pnpm" | "yarn" | "node" | "corepack") => Some(RuntimeKind::Node),
        _ => None,
    })
    .find(|k| *k != app.toolchain);
    let Some(kind) = wanted else {
        return Ok(None);
    };
    let mut found: Vec<_> = installed(state)
        .await?
        .into_iter()
        .filter(|t| t.kind == kind)
        .collect();
    found.sort_by_key(|t| version_key(&t.version));
    Ok(found.last().map(|t| bin_dir(store, kind, &t.version)))
}

pub fn bin_dir(store: &Store, kind: RuntimeKind, version: &str) -> PathBuf {
    let dir = store.dir(kind, version);
    dir.join(by_kind(kind).binary())
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(dir)
}

fn version_key(version: &str) -> Vec<u64> {
    version.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

async fn record(
    state: &State,
    kind: RuntimeKind,
    version: &str,
    path: &Path,
    size: i64,
) -> anyhow::Result<()> {
    let path = path.to_string_lossy();
    sqlx::query!(
        "INSERT INTO toolchains (kind, version, path, size_bytes, installed_at)
         VALUES (?, ?, ?, ?, datetime('now'))
         ON CONFLICT(kind, version) DO UPDATE SET
           path = excluded.path, size_bytes = excluded.size_bytes, installed_at = excluded.installed_at",
        kind,
        version,
        path,
        size
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn ensure(
    state: &State,
    platform: &dyn Platform,
    http: &reqwest::Client,
    store: &Store,
    runtime: &dyn Runtime,
    version: &str,
    target: Target,
    mirrors: &Mirrors,
    mut progress: impl FnMut(Progress),
) -> anyhow::Result<PathBuf> {
    let kind = runtime.kind();
    if !runtime.valid_version(version) {
        bail!("{version} is not a version {kind} can install");
    }
    let dir = store.dir(kind, version);
    if find(state, kind, version).await?.is_some() && dir.join(runtime.binary()).exists() {
        link(platform, runtime, &dir)?;
        progress(Progress::Ready);
        return Ok(dir);
    }

    let partial = store.partial(kind, version);
    let source = runtime
        .source(version, target, &partial, mirrors)
        .with_context(|| format!("{kind} has no toolchain of its own"))?;

    remove_if_present(&partial)?;
    std::fs::create_dir_all(&partial)?;

    let outcome = install(platform, http, &source, &partial, &mut progress).await;
    if let Err(e) = outcome {
        remove_if_present(&partial)?;
        return Err(e);
    }

    let binary = partial.join(runtime.binary());
    if !binary.exists() {
        remove_if_present(&partial)?;
        bail!(
            "the {kind} {version} toolchain did not produce {}",
            runtime.binary()
        );
    }

    link(platform, runtime, &partial)?;
    remove_if_present(&dir)?;
    std::fs::rename(&partial, &dir)
        .with_context(|| format!("moving {} into place", dir.display()))?;
    let size = size_of(&dir)?;
    record(state, kind, version, &dir, size as i64).await?;
    progress(Progress::Ready);
    Ok(dir)
}

async fn install(
    platform: &dyn Platform,
    http: &reqwest::Client,
    source: &Source,
    partial: &Path,
    progress: &mut impl FnMut(Progress),
) -> anyhow::Result<()> {
    match source {
        Source::Archive {
            url,
            format,
            strip_components,
        } => {
            let bytes = download(http, url, progress).await?;
            progress(Progress::Extracting);
            match format {
                ArchiveFormat::TarGz => platform.extract_tar_gz(&bytes, partial, *strip_components),
                ArchiveFormat::Zip => platform.extract_zip(&bytes, partial, *strip_components),
            }
            .with_context(|| format!("unpacking {url}"))?;
        }
        Source::Script {
            url,
            args,
            packages,
        } => {
            let script = download(http, url, progress).await?;
            let text = String::from_utf8(script).context("the installer is not text")?;
            let script_path = partial.join(SCRIPT_NAME);
            platform.write_file(&script_path, &text, 0o755)?;

            let resolved: Vec<String> = packages
                .iter()
                .flat_map(|p| platform.resolve_package(p))
                .collect();
            let names: Vec<&str> = resolved.iter().map(String::as_str).collect();
            progress(Progress::Installing);
            platform
                .install_packages(&names)
                .context("installing the packages the runtime needs")?;

            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            platform
                .run_installer(&script_path, &args, &[])
                .with_context(|| format!("running {url}"))?;
            platform.remove_file(&script_path)?;
        }
    }
    Ok(())
}

async fn download(
    http: &reqwest::Client,
    url: &str,
    progress: &mut impl FnMut(Progress),
) -> anyhow::Result<Vec<u8>> {
    let mut res = http
        .get(url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .with_context(|| format!("downloading {url}"))?;
    let total = res.content_length();
    let mut bytes = Vec::with_capacity(total.unwrap_or(0) as usize);
    progress(Progress::Downloading { received: 0, total });
    while let Some(chunk) = res.chunk().await? {
        bytes.extend_from_slice(&chunk);
        progress(Progress::Downloading {
            received: bytes.len() as u64,
            total,
        });
    }
    Ok(bytes)
}

/// Makes the runtime's links that are missing from `dir`, so toolchains installed before a
/// link was introduced gain it too.
pub fn link(platform: &dyn Platform, runtime: &dyn Runtime, dir: &Path) -> anyhow::Result<()> {
    for (name, target) in runtime.links() {
        let path = dir.join(name);
        if path.symlink_metadata().is_ok() {
            continue;
        }
        platform
            .symlink_swap(Path::new(target), &path)
            .with_context(|| format!("linking {name} beside {}", runtime.binary()))?;
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn size_of(path: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        total += if meta.is_dir() {
            size_of(&entry.path())?
        } else {
            meta.len()
        };
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::RepoTree;
    use crate::github::tests::state;
    use crate::runtime::{Detection, Phase, bun::Bun, node};
    use ferrum_platform::{Arch, FakePlatform};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub struct Downloads {
        pub base: String,
        hits: Arc<AtomicUsize>,
    }

    impl Downloads {
        pub fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }

        pub fn mirrors(&self) -> Mirrors {
            Mirrors {
                node_dist: self.base.clone(),
                bun_releases: self.base.clone(),
                dotnet_script: format!("{}/dotnet-install.sh", self.base),
            }
        }
    }

    fn tarball() -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let data = b"#!node";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "node-v22.11.0-linux-x64/bin/node", &data[..])
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    }

    pub async fn stub_downloads() -> Downloads {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let body = tarball();
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let script = request.lines().next().is_some_and(|l| l.contains(".sh "));
                let payload: &[u8] = if script { b"#!/bin/bash\n" } else { &body };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    payload.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(payload).await;
                let _ = socket.shutdown().await;
            }
        });
        Downloads { base, hits }
    }

    fn target() -> Target {
        Target {
            arch: Arch::X86_64,
            baseline: false,
        }
    }

    #[test]
    fn toolchains_live_under_the_data_directory_and_never_system_wide() {
        let d = Store::default().dir(RuntimeKind::Node, "22.11.0");
        assert!(d.starts_with(crate::DATA_DIR));
        assert_eq!(d, Path::new("/var/lib/ferrum/runtimes/node/22.11.0"));
    }

    #[tokio::test]
    async fn ensure_downloads_once_and_reuses_after() {
        let downloads = stub_downloads().await;
        let (dir, state) = state().await;
        let store = Store::at(dir.path().join("runtimes"));
        let platform = FakePlatform::new();
        let http = crate::http::client();
        let mirrors = downloads.mirrors();

        let first = ensure(
            &state,
            &platform,
            &http,
            &store,
            &node::Node,
            "22.11.0",
            target(),
            &mirrors,
            |_| {},
        )
        .await
        .unwrap();
        let second = ensure(
            &state,
            &platform,
            &http,
            &store,
            &node::Node,
            "22.11.0",
            target(),
            &mirrors,
            |_| {},
        )
        .await
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first, store.dir(RuntimeKind::Node, "22.11.0"));
        assert!(first.join("bin/node").exists());
        assert!(!store.partial(RuntimeKind::Node, "22.11.0").exists());
        assert_eq!(
            downloads.hits(),
            1,
            "an installed toolchain must not be fetched again"
        );
        assert_eq!(platform.calls_matching("extract_tar_gz").len(), 1);

        let listed = installed(&state).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].version, "22.11.0");
        assert!(listed[0].size_bytes > 0);
        assert!(listed[0].installed_at.ends_with('Z'));
    }

    #[tokio::test]
    async fn a_failed_extraction_leaves_no_half_installed_toolchain() {
        let downloads = stub_downloads().await;
        let (dir, state) = state().await;
        let store = Store::at(dir.path().join("runtimes"));
        let platform = FakePlatform::new();
        platform.fail_next("extract_tar_gz");

        let result = ensure(
            &state,
            &platform,
            &crate::http::client(),
            &store,
            &node::Node,
            "22.11.0",
            target(),
            &downloads.mirrors(),
            |_| {},
        )
        .await;
        assert!(result.is_err());
        assert!(
            installed(&state).await.unwrap().is_empty(),
            "a toolchain is recorded only after it exists"
        );
        assert!(!store.partial(RuntimeKind::Node, "22.11.0").exists());
        assert!(!store.dir(RuntimeKind::Node, "22.11.0").exists());
    }

    #[tokio::test]
    async fn progress_reports_bytes_and_the_stated_size() {
        let downloads = stub_downloads().await;
        let (dir, state) = state().await;
        let store = Store::at(dir.path().join("runtimes"));
        let mut seen = Vec::new();
        ensure(
            &state,
            &FakePlatform::new(),
            &crate::http::client(),
            &store,
            &node::Node,
            "22.11.0",
            target(),
            &downloads.mirrors(),
            |p| seen.push(p),
        )
        .await
        .unwrap();
        assert!(matches!(
            seen.first(),
            Some(Progress::Downloading { total: Some(_), .. })
        ));
        assert!(seen.contains(&Progress::Extracting));
        assert_eq!(seen.last(), Some(&Progress::Ready));
        let last_download = seen
            .iter()
            .rev()
            .find_map(|p| match p {
                Progress::Downloading { received, total } => Some((*received, *total)),
                _ => None,
            })
            .unwrap();
        assert_eq!(Some(last_download.0), last_download.1);
    }

    #[tokio::test]
    async fn a_version_the_runtime_cannot_install_is_refused_before_any_download() {
        let downloads = stub_downloads().await;
        let (dir, state) = state().await;
        let store = Store::at(dir.path().join("runtimes"));
        let result = ensure(
            &state,
            &FakePlatform::new(),
            &crate::http::client(),
            &store,
            &node::Node,
            "22",
            target(),
            &downloads.mirrors(),
            |_| {},
        )
        .await;
        assert!(result.is_err());
        assert_eq!(downloads.hits(), 0);
    }

    #[tokio::test]
    async fn a_missing_binary_after_extraction_is_an_error() {
        let downloads = stub_downloads().await;
        let (dir, state) = state().await;
        let store = Store::at(dir.path().join("runtimes"));
        struct WrongBinary;
        impl Runtime for WrongBinary {
            fn kind(&self) -> RuntimeKind {
                RuntimeKind::Bun
            }
            fn detect(&self, _: &RepoTree) -> Option<Detection> {
                None
            }
            fn source(&self, _: &str, _: Target, _: &Path, mirrors: &Mirrors) -> Option<Source> {
                Some(Source::Archive {
                    url: format!("{}/x.tar.gz", mirrors.bun_releases),
                    format: ArchiveFormat::TarGz,
                    strip_components: 1,
                })
            }
            fn binary(&self) -> &'static str {
                "bun"
            }
            fn valid_version(&self, _: &str) -> bool {
                true
            }
            fn env_for(&self, _: Phase, _: &Path, _: Option<u16>) -> Vec<(String, String)> {
                Vec::new()
            }
        }
        let e = ensure(
            &state,
            &FakePlatform::new(),
            &crate::http::client(),
            &store,
            &WrongBinary,
            "1.0.0",
            target(),
            &downloads.mirrors(),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(e.to_string().contains("did not produce bun"), "{e}");
        assert!(installed(&state).await.unwrap().is_empty());
    }

    struct Linked;
    impl Runtime for Linked {
        fn kind(&self) -> RuntimeKind {
            RuntimeKind::Bun
        }
        fn detect(&self, _: &RepoTree) -> Option<Detection> {
            None
        }
        fn source(&self, _: &str, _: Target, _: &Path, mirrors: &Mirrors) -> Option<Source> {
            Some(Source::Archive {
                url: format!("{}/x.tar.gz", mirrors.bun_releases),
                format: ArchiveFormat::TarGz,
                strip_components: 1,
            })
        }
        fn binary(&self) -> &'static str {
            "bin/node"
        }
        fn links(&self) -> &'static [(&'static str, &'static str)] {
            &[("node", "bin/node")]
        }
        fn valid_version(&self, _: &str) -> bool {
            true
        }
        fn env_for(&self, _: Phase, _: &Path, _: Option<u16>) -> Vec<(String, String)> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn a_runtime_s_links_are_made_before_the_toolchain_moves_into_place_and_only_when_missing()
     {
        let downloads = stub_downloads().await;
        let (dir, state) = state().await;
        let store = Store::at(dir.path().join("runtimes"));
        let platform = FakePlatform::new();
        let installed_at = ensure(
            &state,
            &platform,
            &crate::http::client(),
            &store,
            &Linked,
            "1.0.0",
            target(),
            &downloads.mirrors(),
            |_| {},
        )
        .await
        .unwrap();
        let partial = store.partial(RuntimeKind::Bun, "1.0.0");
        assert_eq!(
            platform.calls_matching("symlink_swap"),
            vec![format!(
                "symlink_swap bin/node {}",
                partial.join("node").display()
            )]
        );

        std::os::unix::fs::symlink("bin/node", installed_at.join("node")).unwrap();
        link(&platform, &Linked, &installed_at).unwrap();
        assert_eq!(platform.calls_matching("symlink_swap").len(), 1);
        assert_eq!(Bun.links(), &[("node", "bun"), ("bunx", "bun")]);
    }

    #[tokio::test]
    async fn an_app_whose_commands_name_the_other_tool_gets_its_newest_installed_toolchain() {
        let (_dir, state) = state().await;
        let store = Store::at("/r");
        let app = crate::apps::tests::app("ledger");
        assert_eq!(app.toolchain, RuntimeKind::Node);
        assert_eq!(app.commands.start.as_deref(), Some("bun run start"));
        assert_eq!(extra_for(&state, &store, &app).await.unwrap(), None);

        for version in ["1.9.3", "1.10.0"] {
            record(
                &state,
                RuntimeKind::Bun,
                version,
                &store.dir(RuntimeKind::Bun, version),
                1,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            extra_for(&state, &store, &app).await.unwrap(),
            Some(PathBuf::from("/r/bun/1.10.0"))
        );

        let mut node_only = app.clone();
        node_only.commands.install = Some("npm ci".into());
        node_only.commands.build = None;
        node_only.commands.start = Some("node server.js".into());
        assert_eq!(extra_for(&state, &store, &node_only).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_script_install_writes_the_installer_installs_packages_and_runs_it() {
        let downloads = stub_downloads().await;
        let (dir, state) = state().await;
        let store = Store::at(dir.path().join("runtimes"));
        let platform = FakePlatform::new();
        let partial = store.partial(RuntimeKind::Dotnet, "9.0");
        let e = ensure(
            &state,
            &platform,
            &crate::http::client(),
            &store,
            &crate::runtime::dotnet::Dotnet,
            "9.0",
            target(),
            &downloads.mirrors(),
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(e.to_string().contains("did not produce dotnet"), "{e}");

        let calls = platform.calls();
        let script = format!("write_file {}/install.sh 755", partial.display());
        let run = format!(
            "run_installer {}/install.sh --channel 9.0 --install-dir {} --no-path",
            partial.display(),
            partial.display()
        );
        let written = calls.iter().position(|c| c == &script).unwrap();
        let packages = calls
            .iter()
            .position(|c| c == "install_packages libicu")
            .unwrap();
        let ran = calls.iter().position(|c| c == &run).unwrap();
        assert!(written < packages && packages < ran, "{calls:#?}");
    }
}
