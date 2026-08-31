pub mod acme;
pub mod dns;
pub mod nginx;
pub mod setup;
pub mod state;
pub mod swap;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

pub const LISTEN_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 8443);

pub const DATA_DIR: &str = "/var/lib/ferrum";
pub const ACME_ROOT: &str = "/var/lib/ferrum/acme";
pub const ACME_WEBROOT: &str = "/var/lib/ferrum/acme/.well-known/acme-challenge";
pub const CERTS_DIR: &str = "/var/lib/ferrum/certs";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_managed_path_lives_under_the_data_directory() {
        for p in [ACME_ROOT, ACME_WEBROOT, CERTS_DIR] {
            assert!(p.starts_with(DATA_DIR), "{p}");
        }
    }

    #[test]
    fn the_daemon_is_loopback_only() {
        assert!(LISTEN_ADDR.ip().is_loopback());
        assert_eq!(LISTEN_ADDR.port(), 8443);
    }
}
