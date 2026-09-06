use crate::apps;
use crate::deploy::{self, Outcome};
use crate::metrics::{self, HOST};
use crate::security::{bans, firewall, sshd_or_default};
use crate::state::State;
use crate::{acme, certs, postgres, redis, setup};
use ferrum_platform::Platform;
use ferrum_platform::ubuntu::{NGINX_UNIT, pg_cluster_unit};
use serde::Serialize;

const RENEW_WARN_DAYS: i64 = 30;
const NOT_ENABLED: &str = "not enabled";

#[derive(Debug, Clone, Default)]
pub struct Build {
    pub version: String,
    pub build_id: String,
    pub commit_sha: String,
    pub os: String,
    pub arch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Service {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HostStatus {
    pub hostname: String,
    pub ferrum_version: String,
    pub build_id: String,
    pub commit_sha: String,
    pub os: String,
    pub arch: String,
    pub uptime_secs: u64,
    pub cpu_cores: usize,
    pub cpu_pct: f64,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub swap_used_mb: u64,
    pub swap_total_mb: u64,
    pub disk_used_gb: f64,
    pub disk_total_gb: f64,
    pub certificates_staging: bool,
    pub checklist_hidden: bool,
    pub services: Vec<Service>,
}

const CHECKLIST_HIDDEN: &str = "panel.checklist_hidden";

pub async fn checklist_hidden(state: &State) -> anyhow::Result<bool> {
    Ok(state.get_setting(CHECKLIST_HIDDEN).await?.as_deref() == Some("true"))
}

pub async fn set_checklist_hidden(state: &State, hidden: bool) -> anyhow::Result<()> {
    state
        .set_setting(CHECKLIST_HIDDEN, &hidden.to_string())
        .await
}

fn service(name: &str, ok: bool, detail: impl Into<String>) -> Service {
    Service {
        name: name.to_string(),
        ok,
        detail: detail.into(),
    }
}

fn gb(bytes: u64) -> f64 {
    (bytes as f64 / (1024.0 * 1024.0 * 1024.0) * 10.0).round() / 10.0
}

fn plural(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// "3 hours ago" from a UTC stamp, in the words the panel already uses.
pub fn ago(stamp: &str) -> String {
    let Ok(then) = chrono::DateTime::parse_from_rfc3339(stamp) else {
        return String::new();
    };
    let secs = (chrono::Utc::now() - then.with_timezone(&chrono::Utc)).num_seconds();
    if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        format!("{} min ago", secs / 60)
    } else if secs < 86_400 {
        format!("{} ago", plural((secs / 3600) as usize, "hour"))
    } else {
        format!("{} ago", plural((secs / 86_400) as usize, "day"))
    }
}

pub async fn status(
    state: &State,
    platform: &dyn Platform,
    build: &Build,
) -> anyhow::Result<HostStatus> {
    let mem = platform.proc_meminfo()?;
    let disk = platform.disk_usage(&state.data_dir).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "reading disk usage");
        Default::default()
    });
    let cpu_pct = metrics::latest(state, HOST)
        .await?
        .map(|s| s.cpu_pct)
        .unwrap_or(0.0);
    let staging = matches!(acme::directory(state).await?, acme::Directory::Staging);
    Ok(HostStatus {
        hostname: setup::hostname(state).await?.unwrap_or_default(),
        ferrum_version: build.version.clone(),
        build_id: build.build_id.clone(),
        commit_sha: build.commit_sha.clone(),
        os: build.os.clone(),
        arch: build.arch.clone(),
        uptime_secs: platform.uptime_secs()?,
        cpu_cores: platform.cpu_count(),
        cpu_pct,
        memory_used_mb: mem.total_kb.saturating_sub(mem.available_kb) / 1024,
        memory_total_mb: mem.total_kb / 1024,
        swap_used_mb: mem.swap_total_kb.saturating_sub(mem.swap_free_kb) / 1024,
        swap_total_mb: mem.swap_total_kb / 1024,
        disk_used_gb: gb(disk.used_bytes),
        disk_total_gb: gb(disk.total_bytes),
        certificates_staging: staging,
        checklist_hidden: checklist_hidden(state).await?,
        services: services(state, platform)
            .await?
            .into_iter()
            .map(|s| match s {
                Service { name, ok, detail } if staging && name == "Certificates" => {
                    service(&name, ok, format!("{detail}, staging"))
                }
                s => s,
            })
            .collect(),
    })
}

pub async fn services(state: &State, platform: &dyn Platform) -> anyhow::Result<Vec<Service>> {
    let nginx = platform.service_is_active(NGINX_UNIT);
    let postgres = match platform.postgres_major_installed() {
        Some(major) => {
            let databases = postgres::count(state).await?;
            let running = platform.service_is_active(&pg_cluster_unit(major));
            service(
                "PostgreSQL",
                running,
                if running {
                    format!("{major}, {}", plural(databases, "database"))
                } else {
                    format!("{major} is not running")
                },
            )
        }
        None => service("PostgreSQL", true, "not installed"),
    };
    let instances = redis::list(state).await?.len();
    let redis = service(
        "Redis",
        true,
        if instances == 0 {
            "none".to_string()
        } else {
            plural(instances, "instance")
        },
    );
    Ok(vec![
        service("nginx", nginx, if nginx { "active" } else { "not running" }),
        postgres,
        redis,
        certificates(state, platform).await?,
        deploys(state).await?,
        hardening_row(
            "fail2ban",
            bans::status(state, platform).await.map(|b| {
                b.installed.then(|| {
                    format!(
                        "{}, {} banned",
                        plural(b.jails.len(), "jail"),
                        b.banned.len()
                    )
                })
            }),
        ),
        hardening_row(
            "ufw",
            firewall::status(platform, sshd_or_default(platform)).map(|f| {
                f.enabled
                    .then(|| format!("deny incoming, {}", plural(f.rules.len(), "rule")))
            }),
        ),
    ])
}

/// A tool that is off or not answering asks for a look; neither takes the card down.
fn hardening_row(name: &str, read: anyhow::Result<Option<String>>) -> Service {
    match read {
        Ok(Some(detail)) => service(name, true, detail),
        Ok(None) => service(name, false, NOT_ENABLED),
        Err(e) => {
            tracing::warn!(tool = name, error = %e, "reading hardening status");
            service(name, false, "not answering")
        }
    }
}

async fn certificates(state: &State, platform: &dyn Platform) -> anyhow::Result<Service> {
    let mut nearest: Option<(String, i64)> = None;
    let mut domains = 0;
    for app in apps::list(state).await? {
        for cert in certs::statuses(state, platform, &app).await? {
            domains += 1;
            match cert.status {
                certs::CertStatus::Issued { not_after } => {
                    let days = chrono::DateTime::parse_from_rfc3339(&not_after)
                        .map(|t| (t.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_days())
                        .unwrap_or(0);
                    if nearest.as_ref().is_none_or(|(_, d)| days < *d) {
                        nearest = Some((cert.domain, days));
                    }
                }
                certs::CertStatus::WaitingForDns { .. } => {
                    return Ok(service(
                        "Certificates",
                        false,
                        format!("{} is waiting for DNS", cert.domain),
                    ));
                }
                certs::CertStatus::Failed { .. } => {
                    return Ok(service(
                        "Certificates",
                        false,
                        format!("{} failed to issue", cert.domain),
                    ));
                }
                certs::CertStatus::None => {
                    return Ok(service(
                        "Certificates",
                        false,
                        format!("{} has no certificate yet", cert.domain),
                    ));
                }
            }
        }
    }
    Ok(match nearest {
        None if domains == 0 => service("Certificates", true, "no domains yet"),
        None => service("Certificates", true, "all valid"),
        Some((domain, days)) if days < RENEW_WARN_DAYS => service(
            "Certificates",
            false,
            format!("{domain} renews in {}", plural(days.max(0) as usize, "day")),
        ),
        Some((_, days)) => service(
            "Certificates",
            true,
            format!(
                "all valid, nearest renews in {}",
                plural(days as usize, "day")
            ),
        ),
    })
}

async fn deploys(state: &State) -> anyhow::Result<Service> {
    let Some(last) = deploy::list(state, None, 1).await?.into_iter().next() else {
        return Ok(service("Deploys", true, "none yet"));
    };
    Ok(match last.outcome {
        None => service(
            "Deploys",
            true,
            format!("{} is deploying now", last.app_slug),
        ),
        Some(Outcome::Live) => service(
            "Deploys",
            true,
            format!(
                "last: {} went live {}",
                last.app_slug,
                ago(&last.started_at)
            ),
        ),
        Some(Outcome::RolledBack) => service(
            "Deploys",
            false,
            format!(
                "last: {} was rolled back {}",
                last.app_slug,
                ago(&last.started_at)
            ),
        ),
        Some(Outcome::Failed) => service(
            "Deploys",
            false,
            format!("last: {} failed {}", last.app_slug, ago(&last.started_at)),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acme;
    use crate::apps::tests::{new_app, state};
    use crate::certs::tests::self_signed;
    use crate::deploy::{Commit, Trigger};
    use ferrum_platform::FakePlatform;

    fn build() -> Build {
        Build {
            version: "0.0.1".into(),
            build_id: "b".into(),
            commit_sha: "c".into(),
            os: "Ubuntu 24.04 LTS".into(),
            arch: "x86_64".into(),
        }
    }

    fn detail<'a>(services: &'a [Service], name: &str) -> &'a Service {
        services.iter().find(|s| s.name == name).unwrap()
    }

    #[tokio::test]
    async fn a_fresh_box_is_all_ok_with_nothing_installed() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        setup::set_hostname(&state, "panel.example.com")
            .await
            .unwrap();
        p.set_active("nginx");
        let s = status(&state, &p, &build()).await.unwrap();
        assert_eq!(s.hostname, "panel.example.com");
        assert_eq!(s.os, "Ubuntu 24.04 LTS");
        assert_eq!(s.uptime_secs, 3600);
        assert_eq!(s.cpu_cores, 2);
        assert_eq!(s.cpu_pct, 0.0);
        assert_eq!((s.memory_used_mb, s.memory_total_mb), (1024, 2048));
        assert_eq!((s.swap_used_mb, s.swap_total_mb), (0, 0));
        assert_eq!((s.disk_used_gb, s.disk_total_gb), (20.0, 80.0));
        assert_eq!(detail(&s.services, "nginx").detail, "active");
        assert_eq!(detail(&s.services, "PostgreSQL").detail, "not installed");
        assert_eq!(detail(&s.services, "Redis").detail, "none");
        assert_eq!(detail(&s.services, "Certificates").detail, "no domains yet");
        assert_eq!(detail(&s.services, "Deploys").detail, "none yet");
        assert_eq!(detail(&s.services, "ufw").detail, NOT_ENABLED);
        assert!(
            !detail(&s.services, "ufw").ok,
            "an open box asks for a look"
        );
        assert_eq!(detail(&s.services, "fail2ban").detail, NOT_ENABLED);
        assert!(!detail(&s.services, "fail2ban").ok);
        assert_eq!(
            s.services.iter().filter(|x| !x.ok).count(),
            2,
            "{:?}",
            s.services
        );
        assert_eq!(s.services.len(), 7);
    }

    #[tokio::test]
    async fn the_hardened_rows_count_rules_jails_and_bans_and_survive_a_failing_tool() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        crate::security::firewall::enable(&p).unwrap();
        p.set_active("fail2ban");
        p.set_jails(&[
            "sshd",
            "nginx-http-auth",
            "nginx-botsearch",
            "nginx-limit-req",
        ]);
        p.ban("sshd", "45.148.10.87");
        p.ban("nginx-botsearch", "185.220.101.4");
        let all = services(&state, &p).await.unwrap();
        assert_eq!(detail(&all, "ufw").detail, "deny incoming, 3 rules");
        assert!(detail(&all, "ufw").ok);
        assert_eq!(detail(&all, "fail2ban").detail, "4 jails, 2 banned");
        assert!(detail(&all, "fail2ban").ok);
        p.fail_next("ufw_status");
        let broken = services(&state, &p).await.unwrap();
        assert_eq!(detail(&broken, "ufw").detail, "not answering");
        assert!(!detail(&broken, "ufw").ok);
    }

    #[tokio::test]
    async fn a_short_certificate_and_a_failed_deploy_ask_for_attention_by_name() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        p.set_postgres_major(18);
        p.set_active("postgresql@18-main");
        let mut new = new_app("ledger", &[("/", "main", false)]);
        new.domains = vec!["ledger.example.com".into(), "www.ledger.example.com".into()];
        let app = apps::create(&state, new).await.unwrap();
        for (domain, days) in [("ledger.example.com", 60), ("www.ledger.example.com", 10)] {
            p.write_file(
                &acme::cert_dir(domain).join("fullchain.pem"),
                &self_signed(domain, days),
                0o644,
            )
            .unwrap();
        }
        let first = services(&state, &p).await.unwrap();
        assert!(!detail(&first, "nginx").ok);
        assert_eq!(detail(&first, "PostgreSQL").detail, "18, 0 databases");
        let certs = detail(&first, "Certificates");
        assert!(!certs.ok);
        assert!(
            certs
                .detail
                .starts_with("www.ledger.example.com renews in "),
            "{}",
            certs.detail
        );

        let d = deploy::create(&state, &app, Trigger::Manual, "main", &Commit::default())
            .await
            .unwrap();
        let running = services(&state, &p).await.unwrap();
        assert_eq!(
            detail(&running, "Deploys").detail,
            "ledger is deploying now"
        );
        deploy::finish(&state, &d.id, Outcome::Failed, Some("boom"), None)
            .await
            .unwrap();
        let failed = services(&state, &p).await.unwrap();
        let deploys = detail(&failed, "Deploys");
        assert!(!deploys.ok);
        assert_eq!(deploys.detail, "last: ledger failed just now");
    }

    #[tokio::test]
    async fn a_domain_without_a_certificate_is_named_before_any_expiry() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let mut new = new_app("ledger", &[("/", "main", false)]);
        new.domains = vec!["ledger.example.com".into()];
        apps::create(&state, new).await.unwrap();
        let all = services(&state, &p).await.unwrap();
        assert_eq!(
            detail(&all, "Certificates").detail,
            "ledger.example.com has no certificate yet"
        );
        assert!(!detail(&all, "Certificates").ok);
    }

    #[test]
    fn ago_speaks_the_panel_s_words() {
        let stamp = |secs: i64| {
            (chrono::Utc::now() - chrono::Duration::seconds(secs))
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        };
        assert_eq!(ago(&stamp(5)), "just now");
        assert_eq!(ago(&stamp(300)), "5 min ago");
        assert_eq!(ago(&stamp(3700)), "1 hour ago");
        assert_eq!(ago(&stamp(3 * 86_400)), "3 days ago");
        assert_eq!(ago("garbage"), "");
    }
}
