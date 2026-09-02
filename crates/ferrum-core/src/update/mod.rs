pub mod check;
pub mod verify;

use serde::{Deserialize, Serialize};

pub const REPO: &str = "irixsoft/ferrum";
pub const LATEST_ROUTE: &str = "/repos/irixsoft/ferrum/releases/latest";
pub const AUTO_KEY: &str = "update.auto";
pub const LATEST_KEY: &str = "update.latest";
pub const CHECKED_AT_KEY: &str = "update.checked_at";
pub const SUMS_ASSET: &str = "SHA256SUMS";
pub const SIG_ASSET: &str = "SHA256SUMS.sig";

pub fn target() -> String {
    format!("{}-unknown-linux-musl", std::env::consts::ARCH)
}

pub fn binary_asset(target: &str) -> String {
    format!("ferrum-{target}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Latest {
    pub tag: String,
    pub version: String,
    pub name: String,
    pub notes: String,
    pub security: bool,
    pub published_at: Option<String>,
    pub url: String,
    pub binary_url: String,
    pub sums_url: String,
    pub sig_url: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub running: bool,
    pub step: Option<&'static str>,
    pub error: Option<String>,
    /// The tag installed by this process; a restart clears it with everything else in memory.
    pub applied: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    pub current: String,
    pub latest: Option<Latest>,
    pub available: bool,
    pub checked_at: Option<String>,
    pub auto: bool,
    pub running: bool,
    pub step: Option<&'static str>,
    pub error: Option<String>,
    pub restarting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    #[error("The release has no {0} asset.")]
    NoAsset(String),
    #[error("The release's signature does not verify.")]
    BadSignature,
    #[error("The downloaded binary does not match the release's checksum.")]
    BadChecksum,
    #[error("Ferrum {0} is the latest release.")]
    NotNewer(String),
    #[error("The new binary failed its self-check: {0}")]
    SelfCheck(String),
    #[error("An update is already running.")]
    InProgress,
    #[error("Ferrum {0} is installed and restarts in a moment.")]
    Restarting(String),
    #[error("The download of {0} is larger than the release says.")]
    TooLarge(String),
}

fn parts(version: &str) -> Option<(u64, u64, u64)> {
    let core = version
        .trim()
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()?;
    let mut nums = core.split('.').map(|n| n.parse::<u64>().ok());
    let major = nums.next()??;
    let minor = nums.next()??;
    let patch = nums.next()??;
    if nums.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Compares two versions numerically, with or without a leading `v`; anything unparsable is
/// never newer.
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parts(latest), parts(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically_and_the_v_does_not_matter() {
        assert!(is_newer("v0.1.4", "0.1.3"));
        assert!(!is_newer("v0.1.3", "0.1.3"));
        assert!(!is_newer("v0.2.0", "0.10.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
        assert!(!is_newer("nightly", "0.1.3"));
        assert!(!is_newer("v0.1", "0.0.1"));
    }

    #[test]
    fn the_target_and_asset_names_match_the_release_workflow() {
        assert!(target().ends_with("-unknown-linux-musl"));
        assert_eq!(
            binary_asset("x86_64-unknown-linux-musl"),
            "ferrum-x86_64-unknown-linux-musl"
        );
    }
}
