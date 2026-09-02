pub mod bans;
pub mod firewall;
pub mod ssh;
pub mod updates;

use crate::state::State;
use ferrum_platform::ubuntu::ROOT_AUTHORIZED_KEYS;
use ferrum_platform::{Ban, FirewallRule, KeyFingerprint, Platform, Sshd};
use serde::Serialize;

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

/// The read side tolerates a failing `sshd -T`; every write side reads it strictly.
pub fn sshd_or_default(platform: &dyn Platform) -> Sshd {
    platform.sshd_effective().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "reading the effective sshd config");
        Sshd {
            port: 22,
            password_auth: true,
        }
    })
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
