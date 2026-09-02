pub mod commits;
pub mod contents;
pub mod manifest;
pub mod repos;
pub mod token;
pub mod webhook;

use crate::state::State;
use crate::time;
use serde::Serialize;
use std::sync::{Arc, Mutex};

pub const GITHUB_API: &str = "https://api.github.com";

#[derive(Clone)]
pub struct Api {
    base: String,
    installation: Arc<Mutex<Option<token::Installed>>>,
    fixed_token: Option<String>,
}

impl Default for Api {
    fn default() -> Self {
        Self::at(GITHUB_API)
    }
}

impl Api {
    pub fn at(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            installation: Arc::new(Mutex::new(None)),
            fixed_token: None,
        }
    }

    /// A clone token handed in rather than minted; only tests without a GitHub stub want this.
    pub fn with_fixed_token(mut self, token: impl Into<String>) -> Self {
        self.fixed_token = Some(token.into());
        self
    }

    pub fn anonymous(&self) -> anyhow::Result<octocrab::Octocrab> {
        Ok(octocrab::Octocrab::builder()
            .base_uri(self.base.as_str())?
            .build()?)
    }

    /// Call after connecting or disconnecting, or the cached client keeps the previous app's key.
    pub fn forget(&self) {
        *self
            .installation
            .lock()
            .expect("the cache lock is not poisoned") = None;
    }
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api").field("base", &self.base).finish()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Connection {
    pub app_id: i64,
    pub app_slug: String,
    pub app_name: String,
    pub account: String,
    pub installation_id: Option<i64>,
    pub connected_at: String,
}

#[derive(Debug, Clone)]
pub struct NewConnection {
    pub app_id: i64,
    pub app_slug: String,
    pub app_name: String,
    pub account: String,
    pub private_key: String,
    pub webhook_secret: String,
    pub client_id: String,
    pub client_secret: String,
}

pub async fn save(state: &State, saved: NewConnection) -> anyhow::Result<Connection> {
    let row = sqlx::query!(
        r#"INSERT INTO github_app
             (id, app_id, app_slug, app_name, account, private_key, webhook_secret,
              client_id, client_secret, installation_id, connected_at)
           VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, NULL, datetime('now'))
           ON CONFLICT(id) DO UPDATE SET
             app_id = excluded.app_id,
             app_slug = excluded.app_slug,
             app_name = excluded.app_name,
             account = excluded.account,
             private_key = excluded.private_key,
             webhook_secret = excluded.webhook_secret,
             client_id = excluded.client_id,
             client_secret = excluded.client_secret,
             installation_id = NULL,
             connected_at = excluded.connected_at
           RETURNING app_id AS "app_id!", app_slug AS "app_slug!", app_name AS "app_name!",
                     account AS "account!", installation_id, connected_at AS "connected_at!""#,
        saved.app_id,
        saved.app_slug,
        saved.app_name,
        saved.account,
        saved.private_key,
        saved.webhook_secret,
        saved.client_id,
        saved.client_secret,
    )
    .fetch_one(&state.pool)
    .await?;

    Ok(Connection {
        app_id: row.app_id,
        app_slug: row.app_slug,
        app_name: row.app_name,
        account: row.account,
        installation_id: row.installation_id,
        connected_at: time::utc(row.connected_at),
    })
}

pub async fn load(state: &State) -> anyhow::Result<Option<Connection>> {
    let row = sqlx::query!(
        r#"SELECT app_id AS "app_id!", app_slug AS "app_slug!", app_name AS "app_name!",
                  account AS "account!", installation_id, connected_at AS "connected_at!"
           FROM github_app WHERE id = 1"#
    )
    .fetch_optional(&state.pool)
    .await?;

    Ok(row.map(|r| Connection {
        app_id: r.app_id,
        app_slug: r.app_slug,
        app_name: r.app_name,
        account: r.account,
        installation_id: r.installation_id,
        connected_at: time::utc(r.connected_at),
    }))
}

pub async fn private_key(state: &State) -> anyhow::Result<Option<String>> {
    let row = sqlx::query!(r#"SELECT private_key AS "private_key!" FROM github_app WHERE id = 1"#)
        .fetch_optional(&state.pool)
        .await?;
    Ok(row.map(|r| r.private_key))
}

pub async fn webhook_secret(state: &State) -> anyhow::Result<Option<String>> {
    let row =
        sqlx::query!(r#"SELECT webhook_secret AS "webhook_secret!" FROM github_app WHERE id = 1"#)
            .fetch_optional(&state.pool)
            .await?;
    Ok(row.map(|r| r.webhook_secret))
}

pub async fn set_installation(state: &State, installation_id: i64) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE github_app SET installation_id = ? WHERE id = 1",
        installation_id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn disconnect(state: &State) -> anyhow::Result<()> {
    sqlx::query!("DELETE FROM github_app WHERE id = 1")
        .execute(&state.pool)
        .await?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const TEST_PEM: &str =
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKC\n-----END RSA PRIVATE KEY-----\n";

    pub async fn state() -> (tempfile::TempDir, State) {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(dir.path()).await.unwrap();
        (dir, state)
    }

    pub fn sample() -> NewConnection {
        NewConnection {
            app_id: 12345,
            app_slug: "ferrum-panel-example".into(),
            app_name: "ferrum-panel-example".into(),
            account: "irixsoft".into(),
            private_key: TEST_PEM.into(),
            webhook_secret: "whsec_test".into(),
            client_id: "Iv1.abc".into(),
            client_secret: "cs_abc".into(),
        }
    }

    #[tokio::test]
    async fn a_connection_round_trips_without_exposing_its_secrets() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();

        let loaded = load(&state).await.unwrap().unwrap();
        assert_eq!(loaded.app_name, "ferrum-panel-example");
        assert_eq!(loaded.account, "irixsoft");
        assert!(loaded.installation_id.is_none());

        let json = serde_json::to_string(&loaded).unwrap();
        assert!(!json.contains("PRIVATE KEY"), "{json}");
        assert!(!json.contains("whsec"), "{json}");
        assert!(!json.contains("cs_abc"), "{json}");
    }

    #[tokio::test]
    async fn the_secrets_are_readable_only_on_purpose() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();
        assert!(
            private_key(&state)
                .await
                .unwrap()
                .unwrap()
                .contains("PRIVATE KEY")
        );
        assert_eq!(
            webhook_secret(&state).await.unwrap().as_deref(),
            Some("whsec_test")
        );
    }

    #[tokio::test]
    async fn connecting_again_replaces_the_previous_app() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();
        let mut second = sample();
        second.app_name = "ferrum-panel-second".into();
        save(&state, second).await.unwrap();

        assert_eq!(
            load(&state).await.unwrap().unwrap().app_name,
            "ferrum-panel-second"
        );
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM github_app")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(rows, 1, "there is exactly one GitHub App per install");
    }

    #[tokio::test]
    async fn reconnecting_forgets_the_previous_installation() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();
        set_installation(&state, 4242).await.unwrap();

        save(&state, sample()).await.unwrap();
        assert_eq!(
            load(&state).await.unwrap().unwrap().installation_id,
            None,
            "an installation id belongs to the app that issued it"
        );
    }

    #[tokio::test]
    async fn disconnecting_removes_the_private_key() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();
        disconnect(&state).await.unwrap();

        assert!(load(&state).await.unwrap().is_none());
        assert!(private_key(&state).await.unwrap().is_none());
        assert!(webhook_secret(&state).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_installation_id_is_recorded_after_the_app_is_installed() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();
        set_installation(&state, 4242).await.unwrap();
        assert_eq!(
            load(&state).await.unwrap().unwrap().installation_id,
            Some(4242)
        );
    }

    #[tokio::test]
    async fn a_second_row_cannot_be_inserted() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();
        let forced = sqlx::query(
            "INSERT INTO github_app (id, app_id, app_slug, app_name, account, private_key,
                                     webhook_secret, client_id, client_secret)
             VALUES (2, 1, 's', 'n', 'a', 'k', 'w', 'c', 'cs')",
        )
        .execute(&state.pool)
        .await;
        assert!(forced.is_err(), "the schema must enforce a single app");
    }
}
