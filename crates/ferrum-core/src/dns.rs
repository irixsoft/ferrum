use hickory_resolver::Resolver;
use hickory_resolver::config::{CLOUDFLARE, GOOGLE, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::net::{DnsError, NetError};
use std::net::IpAddr;
use std::time::Duration;

const IP_LOOKUP_URLS: [&str; 2] = ["https://api.ipify.org", "https://ifconfig.me/ip"];

#[derive(Debug, thiserror::Error)]
pub enum DnsLookupError {
    #[error("resolving {host}: {source}")]
    Resolve {
        host: String,
        #[source]
        source: Box<NetError>,
    },
    #[error("could not determine this server's public IP address: {0}")]
    PublicIp(String),
}

pub fn validate_hostname(s: &str) -> Result<String, String> {
    let raw = s.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return Err("Enter a hostname, for example panel.example.com".into());
    }
    if raw.contains("://") || raw.contains('/') {
        return Err("Enter the hostname on its own, with no scheme and no path".into());
    }
    if raw.parse::<IpAddr>().is_ok() {
        return Err(
            "Ferrum needs a domain, not an IP address — a passkey cannot be enrolled against one"
                .into(),
        );
    }

    let host = raw.trim_end_matches('.').to_string();
    if !host.contains('.') {
        return Err(format!(
            "\"{host}\" is a single label; enter a full domain such as panel.example.com"
        ));
    }
    if host.len() > 253 {
        return Err("That hostname is longer than DNS allows".into());
    }
    for label in host.split('.') {
        let valid = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if !valid {
            return Err(format!("\"{host}\" is not a valid hostname"));
        }
    }
    Ok(host)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Match,
    Mismatch {
        found: Vec<IpAddr>,
        expected: IpAddr,
    },
    NoRecord,
}

pub fn classify(found: &[IpAddr], expected: IpAddr) -> Verdict {
    if found.is_empty() {
        return Verdict::NoRecord;
    }
    if found.contains(&expected) {
        return Verdict::Match;
    }
    Verdict::Mismatch {
        found: found.to_vec(),
        expected,
    }
}

pub fn describe(v: &Verdict, host: &str) -> String {
    match v {
        Verdict::Match => format!("{host} points at this server."),
        Verdict::NoRecord => {
            format!("{host} has no A record yet; the change may not have propagated.")
        }
        Verdict::Mismatch { found, expected } => {
            let found = found
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("{host} currently points at {found}; this server is {expected}.")
        }
    }
}

pub async fn resolve_a(host: &str) -> Result<Vec<IpAddr>, DnsLookupError> {
    let mut config = ResolverConfig::udp_and_tcp(&CLOUDFLARE);
    for ns in GOOGLE.udp_and_tcp() {
        config.add_name_server(ns);
    }
    let resolver = Resolver::builder_with_config(config, TokioRuntimeProvider::default())
        .build()
        .map_err(|source| DnsLookupError::Resolve {
            host: host.to_string(),
            source: Box::new(source),
        })?;

    match resolver.lookup_ip(format!("{host}.")).await {
        Ok(lookup) => Ok(lookup.iter().filter(IpAddr::is_ipv4).collect()),
        Err(NetError::Dns(DnsError::NoRecordsFound(_))) => Ok(Vec::new()),
        Err(source) => Err(DnsLookupError::Resolve {
            host: host.to_string(),
            source: Box::new(source),
        }),
    }
}

pub async fn public_ip() -> Result<IpAddr, DnsLookupError> {
    let client = crate::http::client_with_timeout(Duration::from_secs(5));

    let mut last = String::from("no lookup service answered");
    for url in IP_LOOKUP_URLS {
        match client.get(url).send().await {
            Ok(res) => match res.text().await {
                Ok(body) => match body.trim().parse::<IpAddr>() {
                    Ok(ip) => return Ok(ip),
                    Err(e) => last = format!("{url}: {e}"),
                },
                Err(e) => last = format!("{url}: {e}"),
            },
            Err(e) => last = format!("{url}: {e}"),
        }
    }
    Err(DnsLookupError::PublicIp(last))
}

pub async fn verify(host: &str, expected: IpAddr) -> Result<Verdict, DnsLookupError> {
    Ok(classify(&resolve_a(host).await?, expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::IpAddr;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn match_when_the_record_points_here() {
        let v = classify(&[ip("203.0.113.10")], ip("203.0.113.10"));
        assert!(matches!(v, Verdict::Match));
    }

    #[test]
    fn match_when_one_of_several_records_points_here() {
        let v = classify(
            &[ip("198.51.100.1"), ip("203.0.113.10")],
            ip("203.0.113.10"),
        );
        assert!(matches!(v, Verdict::Match));
    }

    #[test]
    fn no_record_is_distinct_from_mismatch() {
        assert!(matches!(
            classify(&[], ip("203.0.113.10")),
            Verdict::NoRecord
        ));
    }

    #[test]
    fn mismatch_message_names_both_addresses() {
        let v = classify(&[ip("198.51.100.1")], ip("203.0.113.10"));
        let msg = describe(&v, "panel.example.com");
        assert!(msg.contains("panel.example.com"), "{msg}");
        assert!(msg.contains("198.51.100.1"), "{msg}");
        assert!(msg.contains("203.0.113.10"), "{msg}");
    }

    #[test]
    fn no_record_message_does_not_claim_a_mismatch() {
        let msg = describe(&Verdict::NoRecord, "panel.example.com");
        assert!(msg.contains("no A record"), "{msg}");
    }

    #[tokio::test]
    #[ignore]
    async fn resolves_a_known_host() {
        let ips = resolve_a("one.one.one.one").await.unwrap();
        assert!(ips.contains(&"1.1.1.1".parse().unwrap()));
    }

    #[tokio::test]
    #[ignore]
    async fn finds_this_hosts_public_address() {
        assert!(public_ip().await.unwrap().is_ipv4());
    }
}
