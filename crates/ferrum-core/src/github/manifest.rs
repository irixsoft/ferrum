use super::{AccountType, Api, NewConnection};
use crate::secret;
use crate::state::State;
use anyhow::Context;
use serde::Deserialize;

pub const NEW_APP_URL: &str = "https://github.com/settings/apps/new";
pub const STATE_TTL_MINUTES: i64 = 15;
const NAME_MAX: usize = 34;

/// GitHub App names are unique across GitHub and at most 34 characters.
pub fn app_name(hostname: &str, organization: Option<&str>) -> String {
    let host = hostname.replace('.', "-");
    let mut name = match organization {
        Some(org) => format!("ferrum-{org}-{host}"),
        None => format!("ferrum-{host}"),
    };
    if name.len() > NAME_MAX {
        name.truncate(NAME_MAX);
    }
    name.trim_end_matches('-').to_string()
}

pub fn manifest(hostname: &str, organization: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "name": app_name(hostname, organization),
        "url": format!("https://{hostname}"),
        "description": "Deploys and manages applications on this server.",
        "hook_attributes": {
            "url": format!("https://{hostname}/api/github/webhook"),
            "active": true,
        },
        "redirect_url": format!("https://{hostname}/api/github/callback"),
        "setup_url": format!("https://{hostname}/settings"),
        "public": false,
        "request_oauth_on_install": false,
        "default_permissions": { "contents": "read", "metadata": "read" },
        "default_events": ["push"],
    })
}

pub fn action(state_value: &str, organization: Option<&str>) -> String {
    match organization {
        Some(org) => {
            format!("https://github.com/organizations/{org}/settings/apps/new?state={state_value}")
        }
        None => format!("{NEW_APP_URL}?state={state_value}"),
    }
}

/// A login is letters, digits and single hyphens, up to 39 characters.
pub fn valid_organization(login: &str) -> bool {
    let bytes = login.as_bytes();
    (1..=39).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        && !login.starts_with('-')
        && !login.ends_with('-')
        && !login.contains("--")
}

pub async fn issue_state(state: &State) -> anyhow::Result<String> {
    let value = secret::generate();
    let hash = secret::hash(&value);
    let ttl = format!("+{STATE_TTL_MINUTES} minutes");

    sqlx::query!(
        "INSERT INTO github_state (hash, expires_at) VALUES (?, datetime('now', ?))",
        hash,
        ttl
    )
    .execute(&state.pool)
    .await?;

    Ok(value)
}

pub async fn consume_state(state: &State, presented: &str) -> anyhow::Result<bool> {
    let hash = secret::hash(presented);
    let row = sqlx::query!(
        "UPDATE github_state SET used_at = datetime('now')
         WHERE hash = ? AND used_at IS NULL AND expires_at > datetime('now')
         RETURNING hash",
        hash
    )
    .fetch_optional(&state.pool)
    .await?;

    Ok(row.is_some())
}

#[derive(Deserialize)]
struct Converted {
    id: i64,
    slug: String,
    name: String,
    owner: Owner,
    pem: String,
    webhook_secret: Option<String>,
    client_id: String,
    client_secret: String,
}

#[derive(Deserialize)]
struct Owner {
    login: String,
    #[serde(rename = "type", default)]
    kind: String,
}

impl Api {
    pub async fn exchange(&self, code: &str) -> anyhow::Result<NewConnection> {
        let route = format!("/app-manifests/{code}/conversions");
        let converted: Converted = self
            .anonymous()?
            .post(route, None::<&()>)
            .await
            .context("exchanging the manifest code for app credentials")?;

        let webhook_secret = converted.webhook_secret.context(
            "github created the app without a webhook secret, so deliveries could not be verified",
        )?;

        let account_type = if converted.owner.kind.eq_ignore_ascii_case("organization") {
            AccountType::Organization
        } else {
            AccountType::User
        };
        Ok(NewConnection {
            app_id: converted.id,
            app_slug: converted.slug,
            app_name: converted.name,
            account: converted.owner.login,
            account_type,
            private_key: converted.pem,
            webhook_secret,
            client_id: converted.client_id,
            client_secret: converted.client_secret,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::state;
    use super::*;

    const HOST: &str = "panel.example.com";

    #[test]
    fn the_manifest_points_every_url_at_this_host() {
        let m = manifest(HOST, None);
        assert_eq!(m["name"], "ferrum-panel-example-com");
        assert_eq!(m["url"], "https://panel.example.com");
        assert_eq!(
            m["hook_attributes"]["url"],
            "https://panel.example.com/api/github/webhook"
        );
        assert_eq!(m["hook_attributes"]["active"], true);
        assert_eq!(
            m["redirect_url"],
            "https://panel.example.com/api/github/callback"
        );
    }

    #[test]
    fn an_organisations_app_is_named_after_it_and_registered_under_it() {
        let m = manifest(HOST, Some("acme"));
        assert_eq!(m["name"], "ferrum-acme-panel-example-com");
        assert_eq!(m["public"], false, "the org's own private App");
        assert_eq!(
            action("abc123", Some("acme")),
            "https://github.com/organizations/acme/settings/apps/new?state=abc123"
        );
        assert_eq!(
            app_name("a-very-long-panel-hostname.example.com", Some("acme")),
            "ferrum-acme-a-very-long-panel-host"
        );
        assert_eq!(
            app_name("x.io", Some("abcdefghijklmnopqrstuvwxy")),
            "ferrum-abcdefghijklmnopqrstuvwxy-x"
        );
        for good in ["acme", "my-org", "a1"] {
            assert!(valid_organization(good), "{good}");
        }
        for bad in ["", "-acme", "acme-", "my--org", "my org", "a/b"] {
            assert!(!valid_organization(bad), "{bad}");
        }
    }

    #[test]
    fn the_app_is_private_and_read_only() {
        let m = manifest(HOST, None);
        assert_eq!(m["public"], false);
        assert_eq!(
            m["default_events"],
            serde_json::json!(["push"]),
            "a tag arrives as a push; the release event is redundant"
        );
        assert!(
            m["default_permissions"]
                .as_object()
                .unwrap()
                .values()
                .all(|v| v == "read"),
            "ferrum never writes to a repository: {m}"
        );
    }

    #[test]
    fn the_state_travels_in_the_action_url() {
        assert_eq!(
            action("abc123", None),
            "https://github.com/settings/apps/new?state=abc123"
        );
    }

    #[tokio::test]
    async fn a_state_is_accepted_once() {
        let (_d, st) = state().await;
        let value = issue_state(&st).await.unwrap();

        assert!(consume_state(&st, &value).await.unwrap());
        assert!(
            !consume_state(&st, &value).await.unwrap(),
            "a replayed state must be refused"
        );
    }

    #[tokio::test]
    async fn an_unknown_state_is_refused() {
        let (_d, st) = state().await;
        assert!(!consume_state(&st, "never-issued").await.unwrap());
    }

    #[tokio::test]
    async fn an_expired_state_is_refused() {
        let (_d, st) = state().await;
        let value = issue_state(&st).await.unwrap();
        sqlx::query("UPDATE github_state SET expires_at = datetime('now', '-1 minute')")
            .execute(&st.pool)
            .await
            .unwrap();

        assert!(!consume_state(&st, &value).await.unwrap());
    }

    #[tokio::test]
    async fn the_plaintext_state_is_never_stored() {
        let (_d, st) = state().await;
        let value = issue_state(&st).await.unwrap();

        let rows: Vec<String> = sqlx::query_scalar("SELECT hash FROM github_state")
            .fetch_all(&st.pool)
            .await
            .unwrap();
        assert!(!rows.iter().any(|h| h.contains(&value)));
    }
}
