use super::{Api, NewConnection};
use crate::secret;
use crate::state::State;
use anyhow::Context;
use serde::Deserialize;

pub const NEW_APP_URL: &str = "https://github.com/settings/apps/new";
pub const STATE_TTL_MINUTES: i64 = 15;

pub fn manifest(hostname: &str) -> serde_json::Value {
    serde_json::json!({
        "name": format!("ferrum-{}", hostname.replace('.', "-")),
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
        "default_events": ["push", "release"],
    })
}

pub fn action(state_value: &str) -> String {
    format!("{NEW_APP_URL}?state={state_value}")
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

        Ok(NewConnection {
            app_id: converted.id,
            app_slug: converted.slug,
            app_name: converted.name,
            account: converted.owner.login,
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
        let m = manifest(HOST);
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
    fn the_app_is_private_and_read_only() {
        let m = manifest(HOST);
        assert_eq!(m["public"], false);
        assert_eq!(m["default_events"], serde_json::json!(["push", "release"]));
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
            action("abc123"),
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
