use std::time::Duration;

const USER_AGENT: &str = concat!("ferrum/", env!("CARGO_PKG_VERSION"));

/// reqwest is built with `rustls-no-provider`, so a client panics unless a provider is installed.
pub fn ensure_tls() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

pub fn client() -> reqwest::Client {
    ensure_tls();
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .build()
        .expect("a reqwest client with no custom TLS settings always builds")
}

pub fn client_with_timeout(timeout: Duration) -> reqwest::Client {
    ensure_tls();
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()
        .expect("a reqwest client with no custom TLS settings always builds")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_client_builds_before_anything_else_has_touched_tls() {
        let _ = client();
        let _ = client_with_timeout(Duration::from_secs(1));
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }
}
