use super::{
    AUTO_KEY, CHECKED_AT_KEY, LATEST_KEY, LATEST_ROUTE, Latest, Progress, SIG_ASSET, SUMS_ASSET,
    Status, UpdateError, binary_asset, is_newer,
};
use crate::github::Api;
use crate::state::State;
use anyhow::Context;
use serde::Deserialize;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    name: Option<String>,
    body: Option<String>,
    html_url: String,
    published_at: Option<String>,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// A line starting `Security:` in the notes, or `(security)` closing the title, marks a release
/// the banner must shout about.
pub fn is_security(name: &str, notes: &str) -> bool {
    name.trim().to_ascii_lowercase().ends_with("(security)")
        || notes.lines().any(|line| {
            line.trim_start()
                .to_ascii_lowercase()
                .starts_with("security:")
        })
}

fn latest_from(release: Release, target: &str) -> Result<Latest, UpdateError> {
    let asset = |name: &str| {
        release
            .assets
            .iter()
            .find(|a| a.name == name)
            .ok_or_else(|| UpdateError::NoAsset(name.to_string()))
    };
    let binary = asset(&binary_asset(target))?;
    let sums = asset(SUMS_ASSET)?;
    let sig = asset(SIG_ASSET)?;
    let name = release.name.clone().unwrap_or_default();
    let notes = release.body.clone().unwrap_or_default();
    Ok(Latest {
        version: release.tag_name.trim_start_matches('v').to_string(),
        security: is_security(&name, &notes),
        name,
        notes,
        tag: release.tag_name,
        published_at: release.published_at,
        url: release.html_url,
        binary_url: binary.browser_download_url.clone(),
        sums_url: sums.browser_download_url.clone(),
        sig_url: sig.browser_download_url.clone(),
        size_bytes: binary.size,
    })
}

pub async fn fetch(api: &Api, target: &str) -> anyhow::Result<Latest> {
    let release: Release = api
        .anonymous()?
        .get(LATEST_ROUTE, None::<&()>)
        .await
        .context("asking GitHub for the latest Ferrum release")?;
    Ok(latest_from(release, target)?)
}

pub async fn stored(state: &State) -> anyhow::Result<Option<Latest>> {
    Ok(state
        .get_setting(LATEST_KEY)
        .await?
        .and_then(|json| serde_json::from_str(&json).ok()))
}

pub async fn remember(state: &State, latest: &Latest) -> anyhow::Result<()> {
    state
        .set_setting(LATEST_KEY, &serde_json::to_string(latest)?)
        .await?;
    state
        .set_setting(
            CHECKED_AT_KEY,
            &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        )
        .await
}

pub async fn auto(state: &State) -> anyhow::Result<bool> {
    Ok(state.get_setting(AUTO_KEY).await?.as_deref() == Some("true"))
}

pub async fn set_auto(state: &State, on: bool) -> anyhow::Result<()> {
    state
        .set_setting(AUTO_KEY, if on { "true" } else { "false" })
        .await
}

pub async fn status(state: &State, current: &str, progress: &Progress) -> anyhow::Result<Status> {
    let latest = stored(state).await?;
    Ok(Status {
        current: current.to_string(),
        available: latest
            .as_ref()
            .is_some_and(|l| is_newer(&l.version, current)),
        latest,
        checked_at: state.get_setting(CHECKED_AT_KEY).await?,
        auto: auto(state).await?,
        running: progress.running,
        step: progress.step,
        error: progress.error.clone(),
        restarting: progress.applied.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::state;

    fn release(assets: &[&str]) -> Release {
        Release {
            tag_name: "v0.1.4".into(),
            name: Some("v0.1.4".into()),
            body: Some("## What's Changed\n* Faster deploys\n".into()),
            html_url: "https://github.com/irixsoft/ferrum/releases/tag/v0.1.4".into(),
            published_at: Some("2026-09-03T10:00:00Z".into()),
            assets: assets
                .iter()
                .map(|name| Asset {
                    name: name.to_string(),
                    browser_download_url: format!(
                        "https://github.com/irixsoft/ferrum/releases/download/v0.1.4/{name}"
                    ),
                    size: 23_000_000,
                })
                .collect(),
        }
    }

    const ALL: [&str; 4] = [
        "ferrum-x86_64-unknown-linux-musl",
        "ferrum-aarch64-unknown-linux-musl",
        "SHA256SUMS",
        "SHA256SUMS.sig",
    ];

    #[test]
    fn a_release_is_read_into_the_three_urls_for_this_target() {
        let latest = latest_from(release(&ALL), "aarch64-unknown-linux-musl").unwrap();
        assert_eq!(latest.tag, "v0.1.4");
        assert_eq!(latest.version, "0.1.4");
        assert!(
            latest
                .binary_url
                .ends_with("/ferrum-aarch64-unknown-linux-musl")
        );
        assert!(latest.sums_url.ends_with("/SHA256SUMS"));
        assert!(latest.sig_url.ends_with("/SHA256SUMS.sig"));
        assert_eq!(latest.size_bytes, 23_000_000);
        assert!(!latest.security);
        assert!(latest.notes.contains("Faster deploys"));
    }

    #[test]
    fn a_release_missing_an_asset_names_it() {
        let missing = latest_from(
            release(&["ferrum-x86_64-unknown-linux-musl", "SHA256SUMS"]),
            "x86_64-unknown-linux-musl",
        )
        .unwrap_err();
        assert_eq!(missing, UpdateError::NoAsset("SHA256SUMS.sig".into()));
        let wrong_arch = latest_from(release(&ALL[1..]), "x86_64-unknown-linux-musl").unwrap_err();
        assert_eq!(
            wrong_arch,
            UpdateError::NoAsset("ferrum-x86_64-unknown-linux-musl".into())
        );
    }

    #[test]
    fn security_releases_are_marked_by_a_note_line_or_the_title() {
        assert!(is_security("v0.1.5 (Security)", ""));
        assert!(is_security(
            "v0.1.5",
            "Fixes\n\nSECURITY: session cookies could be replayed"
        ));
        assert!(is_security("", "  security: yes"));
        assert!(!is_security("v0.1.5", "Improved security of the panel"));
        assert!(!is_security("v0.1.5 security", ""));
    }

    #[tokio::test]
    async fn the_status_reads_what_was_remembered_and_the_progress_it_is_given() {
        let (_d, state) = state().await;
        let empty = status(&state, "0.1.3", &Progress::default()).await.unwrap();
        assert_eq!(empty.current, "0.1.3");
        assert!(empty.latest.is_none());
        assert!(!empty.available && !empty.auto && !empty.running && !empty.restarting);
        assert!(empty.checked_at.is_none());

        let latest = latest_from(release(&ALL), "x86_64-unknown-linux-musl").unwrap();
        remember(&state, &latest).await.unwrap();
        set_auto(&state, true).await.unwrap();
        let progress = Progress {
            running: true,
            step: Some("download"),
            error: None,
            applied: Some("v0.1.4".into()),
        };
        let shown = status(&state, "0.1.3", &progress).await.unwrap();
        assert_eq!(shown.latest.as_ref(), Some(&latest));
        assert!(shown.available && shown.auto && shown.running && shown.restarting);
        assert_eq!(shown.step, Some("download"));
        assert!(shown.checked_at.unwrap().ends_with('Z'));

        let same = status(&state, "0.1.4", &Progress::default()).await.unwrap();
        assert!(!same.available, "the running version is not an update");

        state.set_setting(LATEST_KEY, "{not json").await.unwrap();
        assert!(
            status(&state, "0.1.3", &Progress::default())
                .await
                .unwrap()
                .latest
                .is_none(),
            "a stale shape is ignored rather than fatal"
        );
    }
}
