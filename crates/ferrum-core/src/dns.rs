use hickory_resolver::Resolver;
use hickory_resolver::config::{CLOUDFLARE, GOOGLE, NameServerConfig, ResolverConfig};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::net::{DnsError, NetError, NoRecords};
use hickory_resolver::proto::op::ResponseCode;
use hickory_resolver::proto::rr::RData;
use std::net::IpAddr;
use std::time::Duration;

const IP_LOOKUP_URLS: [&str; 2] = ["https://api.ipify.org", "https://ifconfig.me/ip"];
const TXT_POLL: Duration = Duration::from_secs(5);

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

/// A SERVFAIL arrives as NoRecordsFound too; only NXDOMAIN and NODATA mean the record is absent.
fn is_negative(err: &NetError) -> bool {
    matches!(
        err,
        NetError::Dns(DnsError::NoRecordsFound(NoRecords {
            response_code: ResponseCode::NXDomain | ResponseCode::NoError,
            ..
        }))
    )
}

fn public_resolver(host: &str) -> Result<Resolver<TokioRuntimeProvider>, DnsLookupError> {
    let mut config = ResolverConfig::udp_and_tcp(&CLOUDFLARE);
    for ns in GOOGLE.udp_and_tcp() {
        config.add_name_server(ns);
    }
    Resolver::builder_with_config(config, TokioRuntimeProvider::default())
        .build()
        .map_err(|source| resolve_error(host, source))
}

fn resolve_error(host: &str, source: NetError) -> DnsLookupError {
    DnsLookupError::Resolve {
        host: host.to_string(),
        source: Box::new(source),
    }
}

pub async fn resolve_a(host: &str) -> Result<Vec<IpAddr>, DnsLookupError> {
    let resolver = public_resolver(host)?;
    match resolver.lookup_ip(format!("{host}.")).await {
        Ok(lookup) => Ok(lookup.iter().filter(IpAddr::is_ipv4).collect()),
        Err(source) if is_negative(&source) => Ok(Vec::new()),
        Err(source) => Err(resolve_error(host, source)),
    }
}

/// The name itself, then each parent down to two labels.
pub fn zone_candidates(name: &str) -> Vec<String> {
    let labels: Vec<&str> = name.trim_end_matches('.').split('.').collect();
    (0..labels.len().saturating_sub(1))
        .map(|i| labels[i..].join("."))
        .collect()
}

pub async fn authoritative_ns(name: &str) -> Result<Vec<IpAddr>, DnsLookupError> {
    let resolver = public_resolver(name)?;
    for candidate in zone_candidates(name) {
        let servers: Vec<String> = match resolver.ns_lookup(format!("{candidate}.")).await {
            Ok(lookup) => lookup
                .answers()
                .iter()
                .filter_map(|r| match &r.data {
                    RData::NS(ns) => Some(ns.0.to_ascii()),
                    _ => None,
                })
                .collect(),
            Err(source) if is_negative(&source) => Vec::new(),
            Err(source) => return Err(resolve_error(&candidate, source)),
        };
        if servers.is_empty() {
            continue;
        }
        let mut ips = Vec::new();
        for server in servers {
            let Ok(found) = resolver.lookup_ip(server.as_str()).await else {
                continue;
            };
            for ip in found.iter() {
                if !ips.contains(&ip) {
                    ips.push(ip);
                }
            }
        }
        return Ok(ips);
    }
    Ok(Vec::new())
}

/// Asks only `ips`, with no cache, so an answer is what those servers hold now.
pub async fn txt_at(ips: &[IpAddr], name: &str) -> Result<Vec<String>, DnsLookupError> {
    let servers = ips
        .iter()
        .map(|ip| NameServerConfig::udp_and_tcp(*ip))
        .collect();
    let mut builder = Resolver::builder_with_config(
        ResolverConfig::from_parts(None, vec![], servers),
        TokioRuntimeProvider::default(),
    );
    builder.options_mut().cache_size = 0;
    let resolver = builder
        .build()
        .map_err(|source| resolve_error(name, source))?;
    match resolver
        .txt_lookup(format!("{}.", name.trim_end_matches('.')))
        .await
    {
        Ok(lookup) => Ok(lookup
            .answers()
            .iter()
            .filter_map(|r| match &r.data {
                RData::TXT(txt) => Some(
                    txt.txt_data
                        .iter()
                        .map(|part| String::from_utf8_lossy(part))
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect()),
        Err(source) if is_negative(&source) => Ok(Vec::new()),
        Err(source) => Err(resolve_error(name, source)),
    }
}

/// Waits until every authoritative nameserver of `name` serves `value`, checking every few seconds.
pub async fn txt_visible_on_authoritatives(
    name: &str,
    value: &str,
    deadline: Duration,
) -> anyhow::Result<()> {
    let servers = authoritative_ns(name).await?;
    if servers.is_empty() {
        anyhow::bail!("Found no nameservers for {name}");
    }
    let started = std::time::Instant::now();
    loop {
        let mut missing = 0;
        for ip in &servers {
            let seen = txt_at(&[*ip], name).await.unwrap_or_default();
            if !seen.iter().any(|v| v == value) {
                missing += 1;
            }
        }
        if missing == 0 {
            return Ok(());
        }
        if started.elapsed() >= deadline {
            anyhow::bail!(
                "The TXT record at {name} had not reached {missing} of {} nameservers after {} seconds",
                servers.len(),
                deadline.as_secs()
            );
        }
        tokio::time::sleep(TXT_POLL).await;
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

/// Where lookups go. Tests pin answers so no test reaches a resolver.
#[derive(Debug, Clone)]
pub enum Lookup {
    Public,
    Fixed(Vec<(String, Vec<IpAddr>)>),
}

impl Lookup {
    pub async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, DnsLookupError> {
        match self {
            Self::Public => resolve_a(host).await,
            Self::Fixed(answers) => Ok(answers
                .iter()
                .find(|(name, _)| name == host)
                .map(|(_, ips)| ips.clone())
                .unwrap_or_default()),
        }
    }

    pub async fn verify(&self, host: &str, expected: IpAddr) -> Result<Verdict, DnsLookupError> {
        Ok(classify(&self.resolve(host).await?, expected))
    }
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

    fn no_records(code: ResponseCode) -> NetError {
        NetError::Dns(DnsError::NoRecordsFound(NoRecords::new(
            hickory_resolver::proto::op::Query::default(),
            code,
        )))
    }

    #[test]
    fn an_absent_record_is_an_empty_answer_but_a_servfail_is_an_error() {
        assert!(is_negative(&no_records(ResponseCode::NXDomain)));
        assert!(is_negative(&no_records(ResponseCode::NoError)));
        assert!(!is_negative(&no_records(ResponseCode::ServFail)));
        assert!(!is_negative(&no_records(ResponseCode::Refused)));
        assert!(!is_negative(&NetError::Timeout));
    }

    #[test]
    fn zone_candidates_walk_from_the_name_to_its_registrable_parent() {
        assert_eq!(
            zone_candidates("_acme-challenge.app.example.com."),
            [
                "_acme-challenge.app.example.com",
                "app.example.com",
                "example.com"
            ]
        );
        assert_eq!(zone_candidates("example.com"), ["example.com"]);
    }

    #[tokio::test]
    #[ignore]
    async fn finds_the_authoritative_servers_of_a_known_zone() {
        let ips = authoritative_ns("_acme-challenge.www.cloudflare.com")
            .await
            .unwrap();
        assert!(!ips.is_empty());
        assert!(
            txt_at(&ips, "cloudflare.com")
                .await
                .unwrap()
                .iter()
                .any(|t| t.starts_with("v=spf1"))
        );
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
