use super::{
    Latest, Progress, SIG_ASSET, SUMS_ASSET, Status, UpdateError, binary_asset, check, is_newer,
    verify,
};
use crate::github::Api;
use crate::state::State;
use anyhow::Context;
use ed25519_dalek::VerifyingKey;
use ferrum_platform::Platform;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const UPDATE_DIR: &str = "update";
const SLACK_BYTES: u64 = 1024 * 1024;
const TEXT_LIMIT: u64 = 64 * 1024;

/// The binary this process is, and what replaces it.
#[derive(Clone)]
pub struct Binary {
    pub path: PathBuf,
    pub unit: String,
    pub version: String,
    pub target: String,
    pub key: VerifyingKey,
}

#[derive(Clone)]
pub struct Updater {
    state: State,
    platform: Arc<dyn Platform>,
    api: Api,
    http: reqwest::Client,
    binary: Binary,
    progress: Arc<Mutex<Progress>>,
}

/// Held for the length of one update; dropping it lets the next one start.
struct Claim(Arc<Mutex<Progress>>);

impl Drop for Claim {
    fn drop(&mut self) {
        let mut progress = self.0.lock().unwrap();
        progress.running = false;
        progress.step = None;
    }
}

struct Staging(PathBuf);

impl Staging {
    fn create(data_dir: &Path) -> anyhow::Result<Self> {
        let dir = data_dir.join(UPDATE_DIR);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        Ok(Self(dir))
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Updater {
    pub fn new(
        state: State,
        platform: Arc<dyn Platform>,
        api: Api,
        http: reqwest::Client,
        binary: Binary,
    ) -> Self {
        Self {
            state,
            platform,
            api,
            http,
            binary,
            progress: Arc::default(),
        }
    }

    pub fn progress(&self) -> Progress {
        self.progress.lock().unwrap().clone()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub async fn status(&self) -> anyhow::Result<Status> {
        check::status(&self.state, &self.binary.version, &self.progress()).await
    }

    /// Asks GitHub and remembers the answer; the daily tick and "Check now" both come here.
    pub async fn check(&self) -> anyhow::Result<Status> {
        let latest = check::fetch(&self.api, &self.binary.target).await?;
        check::remember(&self.state, &latest).await?;
        self.status().await
    }

    fn claim(&self, latest: &Latest) -> Result<Claim, UpdateError> {
        let mut progress = self.progress.lock().unwrap();
        if progress.running {
            return Err(UpdateError::InProgress);
        }
        if let Some(applied) = &progress.applied {
            return Err(UpdateError::Restarting(applied.clone()));
        }
        if !is_newer(&latest.version, &self.binary.version) {
            return Err(UpdateError::NotNewer(self.binary.version.clone()));
        }
        progress.running = true;
        progress.error = None;
        progress.step = Some("download");
        Ok(Claim(self.progress.clone()))
    }

    fn step(&self, step: &'static str) {
        self.progress.lock().unwrap().step = Some(step);
        tracing::info!(step, "update");
    }

    /// Runs the whole update and waits for it.
    pub async fn apply(&self, latest: &Latest) -> anyhow::Result<()> {
        let claim = self.claim(latest)?;
        self.run(claim, latest).await
    }

    /// Claims the update now, so the next status shows it running, and finishes in the
    /// background.
    pub fn start(&self, latest: Latest) -> Result<(), UpdateError> {
        let claim = self.claim(&latest)?;
        let updater = self.clone();
        tokio::spawn(async move {
            let _ = updater.run(claim, &latest).await;
        });
        Ok(())
    }

    async fn run(&self, claim: Claim, latest: &Latest) -> anyhow::Result<()> {
        let result = self.install(latest).await;
        {
            let mut progress = self.progress.lock().unwrap();
            match &result {
                Ok(()) => progress.applied = Some(latest.tag.clone()),
                Err(e) => progress.error = Some(describe(e)),
            }
        }
        drop(claim);
        if let Err(e) = &result {
            tracing::warn!(tag = %latest.tag, error = ?e, "update failed");
        }
        result
    }

    async fn install(&self, latest: &Latest) -> anyhow::Result<()> {
        let staging = Staging::create(&self.state.data_dir)?;
        let asset = binary_asset(&self.binary.target);
        tracing::info!(tag = %latest.tag, "downloading");
        let binary = self
            .download(&latest.binary_url, &asset, latest.size_bytes + SLACK_BYTES)
            .await?;
        let sums = self
            .download(&latest.sums_url, SUMS_ASSET, TEXT_LIMIT)
            .await?;
        let sig = self
            .download(&latest.sig_url, SIG_ASSET, TEXT_LIMIT)
            .await?;

        self.step("verify");
        verify::verify(&self.binary.key, &sums, &sig, &binary, &asset)?;
        tracing::info!("signature ok, checksum ok");

        self.step("self-check");
        let staged = staging.0.join("ferrum");
        std::fs::write(&staged, &binary)?;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        let platform = self.platform.clone();
        let path = staged.clone();
        let answer = tokio::task::spawn_blocking(move || platform.self_check(&path))
            .await?
            .map_err(|e| UpdateError::SelfCheck(e.to_string()))?;
        if answer.split_whitespace().nth(1) != Some(latest.version.as_str()) {
            return Err(UpdateError::SelfCheck(format!(
                "it answered {answer:?}, not version {}",
                latest.version
            ))
            .into());
        }
        tracing::info!(answer, "self-check ok");

        self.step("install");
        self.platform
            .install_binary(&staged, &self.binary.path)
            .context("installing the new binary")?;
        tracing::info!(path = %self.binary.path.display(), "installed");

        self.step("restart");
        self.platform
            .restart_later(&self.binary.unit)
            .context("scheduling the restart")?;
        tracing::info!(unit = %self.binary.unit, "restart scheduled");
        Ok(())
    }

    async fn download(&self, url: &str, name: &str, limit: u64) -> anyhow::Result<Vec<u8>> {
        let mut res = self
            .http
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .with_context(|| format!("downloading {name}"))?;
        let mut bytes = Vec::new();
        while let Some(chunk) = res.chunk().await? {
            if (bytes.len() + chunk.len()) as u64 > limit {
                return Err(UpdateError::TooLarge(name.to_string()).into());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    /// One daily tick: refresh, and install only when the toggle is on.
    pub async fn tick(&self) -> anyhow::Result<()> {
        let status = self.check().await?;
        let Some(latest) = status.latest.filter(|_| status.available) else {
            return Ok(());
        };
        if !status.auto || status.restarting {
            return Ok(());
        }
        tracing::info!(tag = %latest.tag, "auto-update");
        self.apply(&latest).await
    }
}

/// An `UpdateError` is a sentence already; anything else gets its chain, cut where GitHub
/// appends a documentation link.
pub fn describe(e: &anyhow::Error) -> String {
    match e.downcast_ref::<UpdateError>() {
        Some(known) => known.to_string(),
        None => format!("{e:#}")
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::apps::tests::state;
    use crate::update::verify::tests::{signing_key, sums_for};
    use ed25519_dalek::{Signer, SigningKey};
    use ferrum_platform::FakePlatform;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    pub const TARGET: &str = "x86_64-unknown-linux-musl";
    const ASSET: &str = "ferrum-x86_64-unknown-linux-musl";

    pub const BASE: &str = "{{base}}";

    /// Answers each path with its bytes, `{{base}}` in a body replaced by the server's own
    /// address; a path under `/slow/` waits first so a test can watch an update in flight.
    pub async fn stub_server(routes: Vec<(String, Vec<u8>)>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes: Vec<(String, Vec<u8>)> = routes
            .into_iter()
            .map(|(path, body)| {
                let text = String::from_utf8_lossy(&body);
                let body = if text.contains(BASE) {
                    text.replace(BASE, &base).into_bytes()
                } else {
                    body
                };
                (path, body)
            })
            .collect();
        let routes = Arc::new(routes);
        tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let routes = routes.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).to_string();
                    let path = request
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .to_string();
                    if path.starts_with("/slow/") {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                    }
                    let (status, body) = match routes.iter().find(|(p, _)| *p == path) {
                        Some((_, body)) => ("200 OK", body.clone()),
                        None => ("404 Not Found", b"{\"message\":\"Not Found\"}".to_vec()),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                    let _ = socket.shutdown().await;
                });
            }
        });
        base
    }

    pub struct Signed {
        pub binary: Vec<u8>,
        pub sums: Vec<u8>,
        pub sig: Vec<u8>,
    }

    pub fn sign_release(key: &SigningKey, binary: &[u8]) -> Signed {
        let sums = sums_for(&[
            (ASSET, binary),
            ("ferrum-aarch64-unknown-linux-musl", b"other"),
        ]);
        let sig = key.sign(&sums).to_bytes().to_vec();
        Signed {
            binary: binary.to_vec(),
            sums,
            sig,
        }
    }

    pub fn release_json(base: &str, tag: &str, size: u64) -> Vec<u8> {
        serde_json::json!({
            "tag_name": tag,
            "name": tag,
            "body": "Notes",
            "html_url": format!("{base}/releases/{tag}"),
            "published_at": "2026-09-03T10:00:00Z",
            "assets": [
                { "name": ASSET, "browser_download_url": format!("{base}/{tag}/{ASSET}"), "size": size },
                { "name": "SHA256SUMS", "browser_download_url": format!("{base}/{tag}/SHA256SUMS"), "size": 200 },
                { "name": "SHA256SUMS.sig", "browser_download_url": format!("{base}/{tag}/SHA256SUMS.sig"), "size": 64 }
            ]
        })
        .to_string()
        .into_bytes()
    }

    fn latest(base: &str, prefix: &str, tag: &str, size: u64) -> Latest {
        Latest {
            tag: tag.into(),
            version: tag.trim_start_matches('v').into(),
            name: tag.into(),
            notes: "Notes".into(),
            security: false,
            published_at: None,
            url: format!("{base}/releases/{tag}"),
            binary_url: format!("{base}{prefix}/{tag}/{ASSET}"),
            sums_url: format!("{base}{prefix}/{tag}/SHA256SUMS"),
            sig_url: format!("{base}{prefix}/{tag}/SHA256SUMS.sig"),
            size_bytes: size,
        }
    }

    struct Rig {
        _dir: tempfile::TempDir,
        updater: Updater,
        platform: Arc<FakePlatform>,
        base: String,
        bin: PathBuf,
        data_dir: PathBuf,
    }

    async fn rig(routes: Vec<(String, Vec<u8>)>, key: &SigningKey, current: &str) -> Rig {
        let (dir, state) = state().await;
        let base = stub_server(routes).await;
        let platform = Arc::new(FakePlatform::new());
        let bin = dir.path().join("bin").join("ferrum");
        let updater = Updater::new(
            state.clone(),
            platform.clone(),
            Api::at(&base),
            crate::http::client(),
            Binary {
                path: bin.clone(),
                unit: "ferrum".into(),
                version: current.into(),
                target: TARGET.into(),
                key: key.verifying_key(),
            },
        );
        Rig {
            data_dir: dir.path().to_path_buf(),
            _dir: dir,
            updater,
            platform,
            base,
            bin,
        }
    }

    fn routes_for(prefix: &str, tag: &str, signed: &Signed) -> Vec<(String, Vec<u8>)> {
        vec![
            (format!("{prefix}/{tag}/{ASSET}"), signed.binary.clone()),
            (format!("{prefix}/{tag}/SHA256SUMS"), signed.sums.clone()),
            (format!("{prefix}/{tag}/SHA256SUMS.sig"), signed.sig.clone()),
        ]
    }

    fn update_calls(platform: &FakePlatform) -> Vec<String> {
        platform
            .calls()
            .into_iter()
            .filter(|c| {
                c.starts_with("self_check")
                    || c.starts_with("install_binary")
                    || c.starts_with("restart_later")
            })
            .collect()
    }

    #[tokio::test]
    async fn a_good_release_is_downloaded_verified_checked_installed_and_the_restart_scheduled() {
        let key = signing_key();
        let signed = sign_release(&key, b"new ferrum binary");
        let r = rig(routes_for("", "v0.1.4", &signed), &key, "0.1.3").await;
        r.platform
            .answer_self_check("ferrum 0.1.4 (build b, commit c)");
        let latest = latest(&r.base, "", "v0.1.4", signed.binary.len() as u64);

        r.updater.apply(&latest).await.unwrap();

        let staged = r.data_dir.join("update").join("ferrum");
        assert_eq!(
            update_calls(&r.platform),
            vec![
                format!("self_check {}", staged.display()),
                format!("install_binary {} {}", staged.display(), r.bin.display()),
                "restart_later ferrum".to_string(),
            ]
        );
        assert_eq!(
            r.platform.installed_binary().as_deref(),
            Some(&b"new ferrum binary"[..])
        );
        assert!(
            !r.data_dir.join("update").exists(),
            "the staging directory is cleaned up"
        );
        let progress = r.updater.progress();
        assert_eq!(progress.applied.as_deref(), Some("v0.1.4"));
        assert!(!progress.running && progress.error.is_none() && progress.step.is_none());
        assert!(r.updater.status().await.unwrap().restarting);

        assert_eq!(
            r.updater.apply(&latest).await.unwrap_err().to_string(),
            "Ferrum v0.1.4 is installed and restarts in a moment."
        );
    }

    #[tokio::test]
    async fn a_bad_signature_stops_before_the_self_check_and_leaves_nothing_behind() {
        let key = signing_key();
        let mut signed = sign_release(&key, b"new ferrum binary");
        signed.sig[3] ^= 0x40;
        let r = rig(routes_for("", "v0.1.4", &signed), &key, "0.1.3").await;
        let latest = latest(&r.base, "", "v0.1.4", signed.binary.len() as u64);

        let e = r.updater.apply(&latest).await.unwrap_err();
        assert_eq!(describe(&e), "The release's signature does not verify.");
        assert!(update_calls(&r.platform).is_empty());
        assert!(!r.data_dir.join("update").exists());
        let progress = r.updater.progress();
        assert_eq!(
            progress.error.as_deref(),
            Some("The release's signature does not verify.")
        );
        assert!(!progress.running && progress.applied.is_none());
        assert!(!r.updater.status().await.unwrap().restarting);
    }

    #[tokio::test]
    async fn a_binary_that_does_not_match_the_sums_is_refused() {
        let key = signing_key();
        let mut signed = sign_release(&key, b"new ferrum binary");
        signed.binary = b"something else entirely".to_vec();
        let r = rig(routes_for("", "v0.1.4", &signed), &key, "0.1.3").await;
        let latest = latest(&r.base, "", "v0.1.4", signed.binary.len() as u64);
        let e = r.updater.apply(&latest).await.unwrap_err();
        assert_eq!(
            describe(&e),
            "The downloaded binary does not match the release's checksum."
        );
        assert!(update_calls(&r.platform).is_empty());
    }

    #[tokio::test]
    async fn a_self_check_naming_another_version_stops_before_the_install() {
        let key = signing_key();
        let signed = sign_release(&key, b"new ferrum binary");
        let r = rig(routes_for("", "v0.1.4", &signed), &key, "0.1.3").await;
        r.platform
            .answer_self_check("ferrum 0.1.3 (build old, commit old)");
        let latest = latest(&r.base, "", "v0.1.4", signed.binary.len() as u64);

        let e = r.updater.apply(&latest).await.unwrap_err();
        assert_eq!(
            describe(&e),
            "The new binary failed its self-check: it answered \"ferrum 0.1.3 (build old, commit old)\", not version 0.1.4"
        );
        assert_eq!(update_calls(&r.platform).len(), 1);
        assert!(!r.data_dir.join("update").exists());

        r.platform.fail_next("self_check");
        let e = r.updater.apply(&latest).await.unwrap_err();
        assert!(
            describe(&e).starts_with("The new binary failed its self-check: self_check"),
            "{e}"
        );
    }

    #[tokio::test]
    async fn a_download_larger_than_the_release_says_is_cut_off() {
        let key = signing_key();
        let big = vec![7u8; 3 * 1024 * 1024];
        let signed = sign_release(&key, &big);
        let r = rig(routes_for("", "v0.1.4", &signed), &key, "0.1.3").await;
        let latest = latest(&r.base, "", "v0.1.4", 1024 * 1024);
        let e = r.updater.apply(&latest).await.unwrap_err();
        assert_eq!(
            describe(&e),
            "The download of ferrum-x86_64-unknown-linux-musl is larger than the release says."
        );
        assert!(update_calls(&r.platform).is_empty());
    }

    #[tokio::test]
    async fn only_one_update_runs_at_a_time_and_an_older_release_is_not_an_update() {
        let key = signing_key();
        let signed = sign_release(&key, b"new ferrum binary");
        let r = rig(routes_for("/slow", "v0.1.4", &signed), &key, "0.1.3").await;
        r.platform
            .answer_self_check("ferrum 0.1.4 (build b, commit c)");
        let latest = latest(&r.base, "/slow", "v0.1.4", signed.binary.len() as u64);

        r.updater.start(latest.clone()).unwrap();
        assert!(r.updater.progress().running);
        assert_eq!(r.updater.progress().step, Some("download"));
        assert_eq!(
            r.updater.apply(&latest).await.unwrap_err().to_string(),
            "An update is already running."
        );
        assert_eq!(
            r.updater.start(latest.clone()),
            Err(UpdateError::InProgress)
        );

        for _ in 0..100 {
            if !r.updater.progress().running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let progress = r.updater.progress();
        assert_eq!(progress.applied.as_deref(), Some("v0.1.4"), "{progress:?}");
        assert_eq!(update_calls(&r.platform).len(), 3);

        let older = rig(vec![], &key, "0.1.5").await;
        assert_eq!(
            older.updater.apply(&latest).await.unwrap_err().to_string(),
            "Ferrum 0.1.5 is the latest release."
        );
    }

    #[tokio::test]
    async fn the_tick_only_remembers_unless_auto_update_is_on_and_then_installs_once() {
        let key = signing_key();
        let signed = sign_release(&key, b"new ferrum binary");
        let mut routes = routes_for("", "v0.1.4", &signed);
        routes.push((
            "/repos/irixsoft/ferrum/releases/latest".into(),
            release_json(BASE, "v0.1.4", signed.binary.len() as u64),
        ));
        let r = rig(routes, &key, "0.1.3").await;
        r.platform
            .answer_self_check("ferrum 0.1.4 (build b, commit c)");

        r.updater.tick().await.unwrap();
        let status = r.updater.status().await.unwrap();
        assert_eq!(
            status.latest.as_ref().map(|l| l.tag.as_str()),
            Some("v0.1.4")
        );
        assert!(status.available && !status.auto && !status.restarting);
        assert!(status.checked_at.is_some());
        assert!(
            update_calls(&r.platform).is_empty(),
            "nothing changes with auto off"
        );

        check::set_auto(r.updater.state(), true).await.unwrap();
        r.updater.tick().await.unwrap();
        assert_eq!(update_calls(&r.platform).len(), 3);
        assert!(r.updater.status().await.unwrap().restarting);

        r.updater.tick().await.unwrap();
        assert_eq!(
            update_calls(&r.platform).len(),
            3,
            "the same tag is not installed twice"
        );
    }
}
