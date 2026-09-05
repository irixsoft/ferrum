pub mod bans;
pub mod firewall;
pub mod persisted;
pub mod ssh;
pub mod updates;

use crate::state::State;
use ferrum_platform::ubuntu::ROOT_AUTHORIZED_KEYS;
use ferrum_platform::{Ban, FirewallRule, KeyFingerprint, Platform, Sshd};
use serde::Serialize;
use std::sync::Mutex;

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("The firewall is already enabled.")]
    AlreadyEnabled,
    #[error(
        "No SSH key is installed. Add your public key to {} and it will appear here.",
        ROOT_AUTHORIZED_KEYS
    )]
    NoKeys,
    #[error("{0} is not an IP address.")]
    BadAddress(String),
    #[error("{0} is not banned.")]
    NotBanned(String),
    #[error("The host refused: {0}")]
    Host(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Firewall {
    pub enabled: bool,
    pub ssh_port: u16,
    pub rules: Vec<FirewallRule>,
    pub persisted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Bans {
    pub installed: bool,
    pub jails: Vec<String>,
    pub banned: Vec<Ban>,
    pub allowlist: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Updates {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Ssh {
    pub port: u16,
    pub password_auth: bool,
    pub keys: Vec<KeyFingerprint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Security {
    pub firewall: Firewall,
    pub bans: Bans,
    pub updates: Updates,
    pub ssh: Ssh,
}

static LAST_SSHD_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// The read side tolerates a failing `sshd -T`; every write side reads it strictly.
pub fn sshd_or_default(platform: &dyn Platform) -> Sshd {
    match platform.sshd_effective() {
        Ok(sshd) => {
            *LAST_SSHD_ERROR.lock().unwrap() = None;
            sshd
        }
        Err(e) => {
            if changed(&mut LAST_SSHD_ERROR.lock().unwrap(), &e.to_string()) {
                tracing::warn!(error = %e, "reading the effective sshd config");
            }
            Sshd {
                port: 22,
                password_auth: true,
            }
        }
    }
}

/// The status is polled, so a failure is logged when it appears or changes, not on every poll.
fn changed(last: &mut Option<String>, error: &str) -> bool {
    if last.as_deref() == Some(error) {
        return false;
    }
    *last = Some(error.to_string());
    true
}

pub async fn status(state: &State, platform: &dyn Platform) -> anyhow::Result<Security> {
    let sshd = sshd_or_default(platform);
    Ok(Security {
        firewall: firewall::status(platform, sshd)?,
        bans: bans::status(state, platform).await?,
        updates: updates::status(platform)?,
        ssh: ssh::status(platform, sshd)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_probe_failure_is_reported_once_until_it_changes() {
        let mut last = None;
        assert!(changed(&mut last, "sshd -T exited with 255"));
        assert!(!changed(&mut last, "sshd -T exited with 255"));
        assert!(!changed(&mut last, "sshd -T exited with 255"));
        assert!(changed(&mut last, "io: permission denied"));
        assert_eq!(last.as_deref(), Some("io: permission denied"));
    }
}
