use super::sigv4::{Request, Signer};
use super::{DnsProvider, Pending, TxtHandle, excerpt};
use anyhow::{Context, bail};
use reqwest::{Method, StatusCode, Url};

const API_VERSION: &str = "2013-04-01";
const REGION: &str = "us-east-1";

pub struct Route53 {
    http: reqwest::Client,
    base: String,
    access_key_id: String,
    secret_access_key: String,
}

impl Route53 {
    pub fn new(
        http: reqwest::Client,
        base: String,
        access_key_id: String,
        secret_access_key: String,
    ) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            access_key_id,
            secret_access_key,
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<String>,
    ) -> anyhow::Result<String> {
        let mut url = Url::parse(&format!("{}{path}", self.base))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        let host = match (url.host_str(), url.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_string(),
            _ => bail!("the Route 53 endpoint has no host"),
        };
        let payload = body.unwrap_or_default();
        let signed = Signer {
            access_key_id: &self.access_key_id,
            secret_access_key: &self.secret_access_key,
            region: REGION,
            service: "route53",
        }
        .sign(
            &Request {
                method: method.as_str(),
                host: &host,
                path,
                query,
                payload: payload.as_bytes(),
            },
            time::OffsetDateTime::now_utc(),
        );
        let mut req = self
            .http
            .request(method, url)
            .header("x-amz-date", signed.amz_date)
            .header("authorization", signed.authorization);
        if !payload.is_empty() {
            req = req.header("content-type", "text/xml").body(payload);
        }
        let res = req
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Route 53 could not be reached: {e}"))?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(text);
        }
        let message = tag(&text, "Message")
            .map(str::to_string)
            .unwrap_or_else(|| excerpt(&text));
        match status {
            StatusCode::FORBIDDEN => bail!(
                "AWS refused the access key: {message}. It needs route53:ListHostedZonesByName and route53:ChangeResourceRecordSets"
            ),
            StatusCode::NOT_FOUND => bail!("Route 53 does not have that hosted zone: {message}"),
            s => bail!("Route 53 answered {s}: {message}"),
        }
    }

    async fn zone_of(&self, fqdn: &str) -> anyhow::Result<String> {
        let labels: Vec<&str> = fqdn.trim_end_matches('.').split('.').collect();
        for i in 0..labels.len().saturating_sub(1) {
            let candidate = labels[i..].join(".");
            let xml = self
                .call(
                    Method::GET,
                    &format!("/{API_VERSION}/hostedzonesbyname"),
                    &[("dnsname", &candidate), ("maxitems", "10")],
                    None,
                )
                .await?;
            let wanted = format!("{candidate}.");
            let found = tags(&xml, "HostedZone").into_iter().find(|zone| {
                tag(zone, "Name") == Some(wanted.as_str())
                    && tag(zone, "PrivateZone") != Some("true")
            });
            if let Some(zone) = found {
                let id = tag(zone, "Id").context("Route 53 listed a zone without an id")?;
                return Ok(id.trim_start_matches("/hostedzone/").to_string());
            }
        }
        bail!("This AWS account has no public Route 53 hosted zone that holds {fqdn}")
    }

    /// The TXT set at the name as Route 53 holds it; a change replaces the whole set, so the
    /// values that are not ours ride along.
    async fn txt_set(&self, zone: &str, fqdn: &str) -> anyhow::Result<TxtSet> {
        let name = format!("{}.", fqdn.trim_end_matches('.'));
        let xml = self
            .call(
                Method::GET,
                &format!("/{API_VERSION}/hostedzone/{zone}/rrset"),
                &[("name", &name), ("type", "TXT"), ("maxitems", "1")],
                None,
            )
            .await?;
        let found = tags(&xml, "ResourceRecordSet")
            .into_iter()
            .find(|set| tag(set, "Name") == Some(name.as_str()) && tag(set, "Type") == Some("TXT"));
        Ok(match found {
            Some(set) => TxtSet {
                ttl: tag(set, "TTL").and_then(|t| t.parse().ok()).unwrap_or(TTL),
                values: tags(set, "Value").into_iter().map(unescape).collect(),
            },
            None => TxtSet::default(),
        })
    }

    async fn change(
        &self,
        zone: &str,
        action: &str,
        fqdn: &str,
        set: &TxtSet,
    ) -> anyhow::Result<()> {
        self.call(
            Method::POST,
            &format!("/{API_VERSION}/hostedzone/{zone}/rrset"),
            &[],
            Some(change_batch(action, fqdn, set)),
        )
        .await?;
        Ok(())
    }
}

const TTL: u32 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
struct TxtSet {
    ttl: u32,
    values: Vec<String>,
}

impl Default for TxtSet {
    fn default() -> Self {
        Self {
            ttl: TTL,
            values: Vec::new(),
        }
    }
}

fn quoted(value: &str) -> String {
    format!("\"{value}\"")
}

fn change_batch(action: &str, fqdn: &str, set: &TxtSet) -> String {
    let records: String = set
        .values
        .iter()
        .map(|v| {
            format!(
                "<ResourceRecord><Value>{}</Value></ResourceRecord>",
                escape(v)
            )
        })
        .collect();
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?>"#,
            r#"<ChangeResourceRecordSetsRequest xmlns="https://route53.amazonaws.com/doc/{v}/">"#,
            "<ChangeBatch><Changes><Change><Action>{action}</Action><ResourceRecordSet>",
            "<Name>{name}.</Name><Type>TXT</Type><TTL>{ttl}</TTL>",
            "<ResourceRecords>{records}</ResourceRecords>",
            "</ResourceRecordSet></Change></Changes></ChangeBatch></ChangeResourceRecordSetsRequest>"
        ),
        v = API_VERSION,
        action = action,
        name = escape(fqdn.trim_end_matches('.')),
        ttl = set.ttl,
        records = records,
    )
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn tags<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        let Some(end) = after.find(&close) else {
            break;
        };
        out.push(&after[..end]);
        rest = &after[end + close.len()..];
    }
    out
}

fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    tags(xml, name).into_iter().next()
}

impl DnsProvider for Route53 {
    fn create_txt<'a>(&'a self, fqdn: &'a str, value: &'a str) -> Pending<'a, TxtHandle> {
        Box::pin(async move {
            let zone = self.zone_of(fqdn).await?;
            let mut set = self.txt_set(&zone, fqdn).await?;
            let ours = quoted(value);
            if !set.values.contains(&ours) {
                set.values.push(ours);
            }
            self.change(&zone, "UPSERT", fqdn, &set).await?;
            Ok(TxtHandle {
                zone,
                record: String::new(),
                fqdn: fqdn.to_string(),
                value: value.to_string(),
            })
        })
    }

    fn delete_txt<'a>(&'a self, handle: TxtHandle) -> Pending<'a, ()> {
        Box::pin(async move {
            let current = self.txt_set(&handle.zone, &handle.fqdn).await?;
            let ours = quoted(&handle.value);
            if !current.values.contains(&ours) {
                return Ok(());
            }
            let rest = TxtSet {
                ttl: current.ttl,
                values: current
                    .values
                    .iter()
                    .filter(|v| **v != ours)
                    .cloned()
                    .collect(),
            };
            if rest.values.is_empty() {
                self.change(&handle.zone, "DELETE", &handle.fqdn, &current)
                    .await
            } else {
                self.change(&handle.zone, "UPSERT", &handle.fqdn, &rest)
                    .await
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dns_providers::stub::Stub;

    const ZONES: &str = r#"<?xml version="1.0"?>
<ListHostedZonesByNameResponse xmlns="https://route53.amazonaws.com/doc/2013-04-01/">
<HostedZones>
<HostedZone><Id>/hostedzone/ZPRIVATE</Id><Name>example.com.</Name><Config><PrivateZone>true</PrivateZone></Config></HostedZone>
<HostedZone><Id>/hostedzone/Z0PUBLIC</Id><Name>example.com.</Name><Config><PrivateZone>false</PrivateZone></Config></HostedZone>
</HostedZones><IsTruncated>false</IsTruncated></ListHostedZonesByNameResponse>"#;

    const NO_ZONES: &str = "<ListHostedZonesByNameResponse><HostedZones></HostedZones></ListHostedZonesByNameResponse>";
    const CHANGED: &str = "<ChangeResourceRecordSetsResponse><ChangeInfo><Status>PENDING</Status></ChangeInfo></ChangeResourceRecordSetsResponse>";

    fn txt_set(values: &[&str]) -> String {
        if values.is_empty() {
            return "<ListResourceRecordSetsResponse><ResourceRecordSets></ResourceRecordSets><IsTruncated>false</IsTruncated></ListResourceRecordSetsResponse>".into();
        }
        let records: String = values
            .iter()
            .map(|v| format!("<ResourceRecord><Value>&quot;{v}&quot;</Value></ResourceRecord>"))
            .collect();
        format!(
            "<ListResourceRecordSetsResponse><ResourceRecordSets><ResourceRecordSet><Name>_acme-challenge.example.com.</Name><Type>TXT</Type><TTL>300</TTL><ResourceRecords>{records}</ResourceRecords></ResourceRecordSet></ResourceRecordSets><IsTruncated>false</IsTruncated></ListResourceRecordSetsResponse>"
        )
    }

    /// Answers the zone walk, then the TXT reads in the order given, then every change.
    fn stub_with(
        reads: Vec<String>,
    ) -> impl Fn(&crate::dns_providers::stub::Seen) -> (u16, String) {
        let reads = std::sync::Mutex::new(reads.into_iter());
        move |req| {
            if req.method == "GET" && req.path.contains("dnsname=example.com&") {
                (200, ZONES.into())
            } else if req.method == "GET" && req.path.contains("hostedzonesbyname") {
                (200, NO_ZONES.into())
            } else if req.method == "GET" {
                (200, reads.lock().unwrap().next().unwrap_or_default())
            } else {
                (200, CHANGED.into())
            }
        }
    }

    #[tokio::test]
    async fn signs_every_call_and_upserts_then_deletes_the_quoted_value() {
        let stub = Stub::serve(stub_with(vec![txt_set(&[]), txt_set(&["v4lue"])])).await;
        let r53 = Route53::new(
            crate::http::client(),
            stub.base.clone(),
            "AKIDEXAMPLE".into(),
            "secret".into(),
        );
        let handle = r53
            .create_txt("_acme-challenge.example.com", "v4lue")
            .await
            .unwrap();
        assert_eq!(handle.zone, "Z0PUBLIC");
        r53.delete_txt(handle).await.unwrap();

        let seen = stub.seen();
        let lines: Vec<String> = seen
            .iter()
            .map(|s| format!("{} {}", s.method, s.path))
            .collect();
        assert_eq!(
            lines,
            [
                "GET /2013-04-01/hostedzonesbyname?dnsname=_acme-challenge.example.com&maxitems=10",
                "GET /2013-04-01/hostedzonesbyname?dnsname=example.com&maxitems=10",
                "GET /2013-04-01/hostedzone/Z0PUBLIC/rrset?name=_acme-challenge.example.com.&type=TXT&maxitems=1",
                "POST /2013-04-01/hostedzone/Z0PUBLIC/rrset",
                "GET /2013-04-01/hostedzone/Z0PUBLIC/rrset?name=_acme-challenge.example.com.&type=TXT&maxitems=1",
                "POST /2013-04-01/hostedzone/Z0PUBLIC/rrset",
            ]
        );
        for s in &seen {
            let auth = s.header("authorization").unwrap();
            assert!(
                auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/")
                    && auth.contains("/us-east-1/route53/aws4_request")
                    && auth.contains("SignedHeaders=host;x-amz-date"),
                "{auth}"
            );
            assert!(s.header("x-amz-date").unwrap().ends_with('Z'));
        }
        assert!(seen[3].body.contains("<Action>UPSERT</Action>"));
        assert!(
            seen[3]
                .body
                .contains("<Name>_acme-challenge.example.com.</Name>")
        );
        assert!(seen[3].body.contains("<TTL>60</TTL>"));
        assert!(seen[3].body.contains("<Value>&quot;v4lue&quot;</Value>"));
        assert!(seen[5].body.contains("<Action>DELETE</Action>"));
        assert!(
            seen[5].body.contains("<TTL>300</TTL>"),
            "a delete must match the set as Route 53 holds it: {}",
            seen[5].body
        );
        assert_eq!(seen[3].header("content-type"), Some("text/xml"));
    }

    #[tokio::test]
    async fn a_value_someone_else_put_at_the_name_rides_along_and_stays() {
        let stub = Stub::serve(stub_with(vec![
            txt_set(&["other"]),
            txt_set(&["other", "v4lue"]),
        ]))
        .await;
        let r53 = Route53::new(
            crate::http::client(),
            stub.base.clone(),
            "AKIDEXAMPLE".into(),
            "secret".into(),
        );
        let handle = r53
            .create_txt("_acme-challenge.example.com", "v4lue")
            .await
            .unwrap();
        r53.delete_txt(handle).await.unwrap();

        let posts: Vec<String> = stub
            .seen()
            .into_iter()
            .filter(|s| s.method == "POST")
            .map(|s| s.body)
            .collect();
        assert_eq!(posts.len(), 2);
        assert!(posts[0].contains("<Action>UPSERT</Action>"));
        assert!(posts[0].contains("<TTL>300</TTL>"));
        assert!(posts[0].contains("<Value>&quot;other&quot;</Value>"));
        assert!(posts[0].contains("<Value>&quot;v4lue&quot;</Value>"));
        assert!(
            posts[1].contains("<Action>UPSERT</Action>"),
            "the other value stays, so the set is rewritten, not deleted: {}",
            posts[1]
        );
        assert!(posts[1].contains("<Value>&quot;other&quot;</Value>"));
        assert!(!posts[1].contains("v4lue"));
    }

    #[tokio::test]
    async fn a_refused_key_carries_the_aws_message() {
        let stub = Stub::serve(|_| {
            (
                403,
                "<ErrorResponse><Error><Code>SignatureDoesNotMatch</Code><Message>The request signature we calculated does not match</Message></Error></ErrorResponse>".into(),
            )
        })
        .await;
        let r53 = Route53::new(
            crate::http::client(),
            stub.base.clone(),
            "A".into(),
            "S".into(),
        );
        let err = r53
            .create_txt("_acme-challenge.example.com", "v")
            .await
            .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("AWS refused the access key"), "{text}");
        assert!(text.contains("signature we calculated"), "{text}");
    }

    #[test]
    fn reads_nested_tags() {
        let zones = tags(ZONES, "HostedZone");
        assert_eq!(zones.len(), 2);
        assert_eq!(tag(zones[1], "Id"), Some("/hostedzone/Z0PUBLIC"));
    }
}
