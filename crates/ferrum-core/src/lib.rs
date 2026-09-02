pub mod acme;
pub mod apps;
pub mod certs;
pub mod credentials;
pub mod deploy;
pub mod detect;
pub mod dns;
pub mod enrollment;
pub mod github;
pub mod http;
pub mod metrics;
pub mod nginx;
pub mod postgres;
pub mod redis;
pub mod runtime;
mod secret;
pub mod sessions;
pub mod setup;
pub mod state;
pub mod swap;
pub mod time;
pub mod tokens;
pub mod users;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub const LISTEN_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8443);

pub const FERRUM_UNIT: &str = "ferrum";

pub const DATA_DIR: &str = "/var/lib/ferrum";
pub const ACME_ROOT: &str = "/var/lib/ferrum/acme";
pub const ACME_WEBROOT: &str = "/var/lib/ferrum/acme/.well-known/acme-challenge";
pub const CERTS_DIR: &str = "/var/lib/ferrum/certs";
pub const APPS_DIR: &str = "/var/lib/ferrum/apps";
pub const REDIS_DIR: &str = "/var/lib/ferrum/redis";
pub const SNAPSHOTS_DIR: &str = "/var/lib/ferrum/snapshots";
pub const PAGES_DIR: &str = "/var/lib/ferrum/pages";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_managed_path_lives_under_the_data_directory() {
        for p in [
            ACME_ROOT,
            ACME_WEBROOT,
            CERTS_DIR,
            APPS_DIR,
            REDIS_DIR,
            SNAPSHOTS_DIR,
            PAGES_DIR,
            runtime::toolchain::RUNTIMES_DIR,
        ] {
            assert!(p.starts_with(DATA_DIR), "{p}");
        }
    }

    #[test]
    fn the_daemon_is_loopback_only() {
        assert!(LISTEN_ADDR.ip().is_loopback());
        assert_eq!(LISTEN_ADDR.port(), 8443);
    }
}
