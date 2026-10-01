use super::processes::NewProcess;
use super::{App, AppError, NewRoute, invalid, is_unique_violation};
use crate::dns::validate_hostname;
use crate::state::State;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

pub const WILDCARD_NEEDS_PROVIDER: &str = "A wildcard needs a DNS provider under Settings first.";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum Job {
    #[default]
    Serve,
    Redirect,
}

/// A name the app answers for: `target` is a process for a served name, a served name for a
/// redirect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Domain {
    pub domain: String,
    pub job: Job,
    pub target: String,
    pub primary: bool,
    pub wildcard: bool,
    pub dns_provider_id: Option<String>,
}

impl Domain {
    pub fn serves(&self) -> bool {
        self.job == Job::Serve
    }
}

/// A bare string is a served name for the process behind `/`; with no row marked primary the
/// first served name becomes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "Wire")]
pub struct NewDomain {
    pub domain: String,
    pub job: Job,
    pub target: String,
    pub primary: bool,
    pub dns_provider_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Wire {
    Name(String),
    Row {
        domain: String,
        #[serde(default)]
        job: Job,
        #[serde(default)]
        target: String,
        #[serde(default)]
        primary: bool,
        #[serde(default)]
        dns_provider_id: Option<String>,
    },
}

impl From<Wire> for NewDomain {
    fn from(wire: Wire) -> Self {
        match wire {
            Wire::Name(domain) => domain.as_str().into(),
            Wire::Row {
                domain,
                job,
                target,
                primary,
                dns_provider_id,
            } => Self {
                domain,
                job,
                target,
                primary,
                dns_provider_id,
            },
        }
    }
}

impl From<&str> for NewDomain {
    fn from(domain: &str) -> Self {
        Self {
            domain: domain.into(),
            job: Job::Serve,
            target: String::new(),
            primary: false,
            dns_provider_id: None,
        }
    }
}

impl From<&Domain> for NewDomain {
    fn from(d: &Domain) -> Self {
        Self {
            domain: d.domain.clone(),
            job: d.job,
            target: d.target.clone(),
            primary: d.primary,
            dns_provider_id: d.dns_provider_id.clone(),
        }
    }
}

/// What a PATCH may change on one name; a field left out keeps its value.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DomainChange {
    pub job: Option<Job>,
    pub target: Option<String>,
    pub primary: Option<bool>,
    pub dns_provider_id: Option<String>,
}

pub fn is_wildcard(domain: &str) -> bool {
    domain.starts_with("*.")
}

fn clean_name(raw: &str) -> Result<String, String> {
    let raw = raw.trim().to_ascii_lowercase();
    match raw.strip_prefix("*.") {
        Some(rest) => validate_hostname(rest).map(|host| format!("*.{host}")),
        None => validate_hostname(&raw),
    }
}

/// The list as it is stored: names cleaned, a served name without a target pointed at the
/// process behind `/`, and exactly one primary.
pub fn settle(
    domains: &[NewDomain],
    processes: &[NewProcess],
    routes: &[NewRoute],
) -> Result<Vec<NewDomain>, AppError> {
    let main = routes
        .iter()
        .find(|r| r.path == "/")
        .map(|r| r.process.clone())
        .unwrap_or_else(|| super::processes::WEB.into());
    let mut settled: Vec<NewDomain> = Vec::with_capacity(domains.len());
    for d in domains {
        let domain = clean_name(&d.domain).map_err(invalid)?;
        if settled.iter().any(|s| s.domain == domain) {
            return Err(invalid(format!("{domain} is listed twice.")));
        }
        let target = d.target.trim().to_ascii_lowercase();
        let target = match d.job {
            Job::Serve if target.is_empty() => main.clone(),
            _ => target,
        };
        settled.push(NewDomain {
            domain,
            job: d.job,
            target,
            primary: d.primary,
            dns_provider_id: d.dns_provider_id.clone().filter(|id| !id.trim().is_empty()),
        });
    }

    for d in &settled {
        match d.job {
            Job::Serve => {
                let Some(process) = processes.iter().find(|p| p.name == d.target) else {
                    return Err(invalid(format!(
                        "{} points at {}, which is not one of the processes.",
                        d.domain, d.target
                    )));
                };
                if !process.is_folder() && !process.port {
                    return Err(invalid(format!(
                        "{} points at {}, which has no port to receive it.",
                        d.domain, d.target
                    )));
                }
            }
            Job::Redirect => {
                if d.target == d.domain {
                    return Err(invalid(format!("{} cannot redirect to itself.", d.domain)));
                }
                let served = settled.iter().any(|s| {
                    s.domain == d.target && s.job == Job::Serve && !is_wildcard(&s.domain)
                });
                if !served {
                    return Err(invalid(format!(
                        "{} redirects to {}, which is not a name this application serves.",
                        d.domain, d.target
                    )));
                }
                if d.primary {
                    return Err(invalid(format!(
                        "{} redirects, so it cannot be the primary domain.",
                        d.domain
                    )));
                }
            }
        }
        if is_wildcard(&d.domain) && d.dns_provider_id.is_none() {
            return Err(invalid(WILDCARD_NEEDS_PROVIDER));
        }
    }

    match settled.iter().filter(|d| d.primary).count() {
        0 => {
            let pick = settled
                .iter()
                .position(|d| d.job == Job::Serve && !is_wildcard(&d.domain))
                .or_else(|| settled.iter().position(|d| d.job == Job::Serve));
            if let Some(i) = pick {
                settled[i].primary = true;
            }
        }
        1 => {}
        _ => return Err(invalid("Only one domain can be primary.")),
    }
    Ok(settled)
}

/// Adds the name, or replaces the row of the same name; a new primary takes over from the old.
pub fn put(current: &[Domain], new: NewDomain) -> Vec<NewDomain> {
    let name = new.domain.trim().to_ascii_lowercase();
    let mut list: Vec<NewDomain> = current.iter().map(NewDomain::from).collect();
    if new.primary {
        for d in &mut list {
            d.primary = false;
        }
    }
    match list.iter_mut().find(|d| d.domain == name) {
        Some(row) => *row = new,
        None => list.push(new),
    }
    list
}

pub fn change(
    current: &[Domain],
    name: &str,
    change: DomainChange,
) -> Result<Vec<NewDomain>, AppError> {
    let Some(row) = current.iter().find(|d| d.domain == name) else {
        return Err(AppError::DomainNotFound(name.to_string()));
    };
    let mut new = NewDomain::from(row);
    if let Some(job) = change.job {
        new.job = job;
    }
    if let Some(target) = change.target {
        new.target = target;
    }
    if let Some(primary) = change.primary {
        new.primary = primary;
    }
    if let Some(id) = change.dns_provider_id {
        new.dns_provider_id = Some(id);
    }
    Ok(put(current, new))
}

pub fn remove(current: &[Domain], name: &str) -> Result<Vec<NewDomain>, AppError> {
    let Some(row) = current.iter().find(|d| d.domain == name) else {
        return Err(AppError::DomainNotFound(name.to_string()));
    };
    if row.primary && current.iter().any(|d| d.domain != name && d.serves()) {
        return Err(invalid(format!(
            "{name} is the primary domain; make another served name primary before removing it."
        )));
    }
    Ok(current
        .iter()
        .filter(|d| d.domain != name)
        .map(NewDomain::from)
        .collect())
}

/// Every wildcard names a provider that exists.
pub(super) async fn check_providers(
    tx: &mut Transaction<'_, Sqlite>,
    domains: &[NewDomain],
) -> anyhow::Result<()> {
    for d in domains.iter().filter(|d| is_wildcard(&d.domain)) {
        let id = d.dns_provider_id.as_deref().unwrap_or_default();
        let found = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!: i64" FROM dns_providers WHERE id = ?"#,
            id
        )
        .fetch_one(&mut **tx)
        .await?;
        if found == 0 {
            return Err(invalid(WILDCARD_NEEDS_PROVIDER).into());
        }
    }
    Ok(())
}

/// Replaces the app's names with a settled list.
pub(super) async fn write(
    tx: &mut Transaction<'_, Sqlite>,
    app_id: &str,
    domains: &[NewDomain],
) -> anyhow::Result<()> {
    sqlx::query!("DELETE FROM app_domains WHERE app_id = ?", app_id)
        .execute(&mut **tx)
        .await?;
    for (position, d) in domains.iter().enumerate() {
        let position = position as i64;
        let inserted = sqlx::query!(
            "INSERT INTO app_domains (domain, app_id, position, job, target, primary_domain, dns_provider_id)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            d.domain,
            app_id,
            position,
            d.job,
            d.target,
            d.primary,
            d.dns_provider_id,
        )
        .execute(&mut **tx)
        .await;
        if let Err(e) = inserted {
            if is_unique_violation(&e) {
                return Err(invalid(format!(
                    "{} already belongs to another application.",
                    d.domain
                ))
                .into());
            }
            return Err(e.into());
        }
    }
    Ok(())
}

/// Primary first, then in the order given.
pub async fn of(state: &State, app_id: &str) -> anyhow::Result<Vec<Domain>> {
    let rows = sqlx::query!(
        r#"SELECT domain AS "domain!", job AS "job!: Job", target, primary_domain AS "primary!: bool",
                  dns_provider_id
           FROM app_domains WHERE app_id = ? ORDER BY primary_domain DESC, position"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Domain {
            wildcard: is_wildcard(&r.domain),
            domain: r.domain,
            job: r.job,
            target: r.target.unwrap_or_default(),
            primary: r.primary,
            dns_provider_id: r.dns_provider_id,
        })
        .collect())
}

impl App {
    pub fn primary_domain(&self) -> Option<&str> {
        self.domains
            .iter()
            .find(|d| d.primary)
            .map(|d| d.domain.as_str())
    }

    pub fn served_domains(&self) -> impl Iterator<Item = &Domain> {
        self.domains.iter().filter(|d| d.serves())
    }

    pub fn domain(&self, name: &str) -> Option<&Domain> {
        self.domains.iter().find(|d| d.domain == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::apps::{AppChanges, create, update};

    fn row(domain: &str, job: Job, target: &str) -> NewDomain {
        NewDomain {
            domain: domain.into(),
            job,
            target: target.into(),
            primary: false,
            dns_provider_id: None,
        }
    }

    #[test]
    fn a_bare_string_and_an_object_both_read_as_a_name() {
        let list: Vec<NewDomain> = serde_json::from_str(
            r#"["shop.example.com", {"domain":"www.shop.example.com","job":"redirect","target":"shop.example.com"}]"#,
        )
        .unwrap();
        assert_eq!(list[0], NewDomain::from("shop.example.com"));
        assert_eq!(
            list[1],
            row("www.shop.example.com", Job::Redirect, "shop.example.com")
        );
    }

    #[test]
    fn the_first_served_name_becomes_primary_and_a_bare_name_serves_the_root_process() {
        let new = new_app("shop", &[("/", "main", false), ("/admin", "admin", false)]);
        let settled = settle(
            &[
                row("www.shop.example.com", Job::Redirect, "shop.example.com"),
                " Shop.Example.com ".into(),
                row("admin.shop.example.com", Job::Serve, "admin"),
            ],
            &new.processes,
            &new.routes,
        )
        .unwrap();
        assert_eq!(settled[1].domain, "shop.example.com");
        assert_eq!(settled[1].target, "web");
        let primaries: Vec<&str> = settled
            .iter()
            .filter(|d| d.primary)
            .map(|d| d.domain.as_str())
            .collect();
        assert_eq!(primaries, ["shop.example.com"]);
    }

    #[test]
    fn a_name_that_cannot_be_served_is_refused_with_a_reason() {
        let mut new = new_app("shop", &[("/", "main", false)]);
        new.processes
            .push(NewProcess::worker("jobs", "bun run jobs"));
        let refused = |domains: &[NewDomain]| {
            settle(domains, &new.processes, &new.routes)
                .unwrap_err()
                .to_string()
        };
        assert!(
            refused(&[row("a.example.com", Job::Serve, "nobody")])
                .contains("not one of the processes")
        );
        assert!(refused(&[row("a.example.com", Job::Serve, "jobs")]).contains("no port"));
        assert!(
            refused(&[row("a.example.com", Job::Redirect, "a.example.com")]).contains("itself")
        );
        assert!(
            refused(&[
                "a.example.com".into(),
                row("b.example.com", Job::Redirect, "a.example.com"),
                row("c.example.com", Job::Redirect, "b.example.com"),
            ])
            .contains("not a name this application serves"),
            "a redirect never points at a redirect"
        );
        let mut primary_redirect = row("b.example.com", Job::Redirect, "a.example.com");
        primary_redirect.primary = true;
        assert!(refused(&["a.example.com".into(), primary_redirect]).contains("primary"));
        assert_eq!(refused(&["*.example.com".into()]), WILDCARD_NEEDS_PROVIDER);
        assert!(refused(&["a.*.example.com".into()]).contains("not a valid hostname"));
        assert!(refused(&["a.example.com".into(), "A.example.com".into()]).contains("twice"));
    }

    #[tokio::test]
    async fn a_wildcard_needs_a_provider_that_exists() {
        let (_d, state) = state().await;
        let mut new = new_app("shop", &[("/", "main", false)]);
        let mut wildcard = NewDomain::from("*.shop.example.com");
        wildcard.dns_provider_id = Some("cf".into());
        new.domains.push(wildcard);
        let e = create(&state, new.clone()).await.unwrap_err();
        assert_eq!(e.to_string(), WILDCARD_NEEDS_PROVIDER);

        sqlx::query("INSERT INTO dns_providers (id, name, kind, credentials) VALUES ('cf', 'Cloudflare', 'cloudflare', 'x')")
            .execute(&state.pool)
            .await
            .unwrap();
        let app = create(&state, new).await.unwrap();
        let wildcard = app.domain("*.shop.example.com").unwrap();
        assert!(wildcard.wildcard);
        assert_eq!(wildcard.dns_provider_id.as_deref(), Some("cf"));
        assert_eq!(app.primary_domain(), Some("shop.example.com"));
    }

    #[tokio::test]
    async fn rows_round_trip_and_the_primary_comes_first() {
        let (_d, state) = state().await;
        let new = new_app("shop", &[("/", "main", false), ("/admin", "admin", false)]);
        let app = create(&state, new).await.unwrap();
        let mut admin = row("admin.shop.example.com", Job::Serve, "admin");
        admin.primary = true;
        let list = put(&app.domains, admin);
        let list = put(
            &crate::apps::tests::rows(&list),
            row("www.shop.example.com", Job::Redirect, "shop.example.com"),
        );
        let updated = update(
            &state,
            "shop",
            AppChanges {
                domains: Some(list),
                ..AppChanges::default()
            },
        )
        .await
        .unwrap();
        let shape: Vec<(&str, Job, &str, bool)> = updated
            .domains
            .iter()
            .map(|d| (d.domain.as_str(), d.job, d.target.as_str(), d.primary))
            .collect();
        assert_eq!(
            shape,
            [
                ("admin.shop.example.com", Job::Serve, "admin", true),
                ("shop.example.com", Job::Serve, "web", false),
                (
                    "www.shop.example.com",
                    Job::Redirect,
                    "shop.example.com",
                    false
                ),
            ]
        );
        assert_eq!(updated.served_domains().count(), 2);
    }

    #[test]
    fn the_primary_stays_while_other_served_names_exist() {
        let rows = crate::apps::tests::rows(&[
            {
                let mut d = NewDomain::from("a.example.com");
                d.primary = true;
                d.target = "web".into();
                d
            },
            row("b.example.com", Job::Serve, "web"),
        ]);
        assert!(remove(&rows, "a.example.com").is_err());
        assert_eq!(remove(&rows, "b.example.com").unwrap().len(), 1);
        assert!(matches!(
            remove(&rows, "c.example.com"),
            Err(AppError::DomainNotFound(_))
        ));
        let moved = change(
            &rows,
            "b.example.com",
            DomainChange {
                primary: Some(true),
                ..DomainChange::default()
            },
        )
        .unwrap();
        assert_eq!(
            moved
                .iter()
                .filter(|d| d.primary)
                .map(|d| d.domain.as_str())
                .collect::<Vec<_>>(),
            ["b.example.com"]
        );
    }
}
