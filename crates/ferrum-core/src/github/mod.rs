pub mod commits;
pub mod contents;
pub mod manifest;
pub mod refs;
pub mod repos;
pub mod token;
pub mod webhook;

use crate::state::State;
use crate::{secrets, time};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const GITHUB_API: &str = "https://api.github.com";

#[derive(Clone)]
pub struct Api {
    base: String,
    installations: Arc<Mutex<HashMap<i64, token::Installed>>>,
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
            installations: Arc::new(Mutex::new(HashMap::new())),
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

    /// Call after connecting or disconnecting, or a cached client keeps a previous app's key.
    pub fn forget(&self) {
        self.installations
            .lock()
            .expect("the cache lock is not poisoned")
            .clear();
    }
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api").field("base", &self.base).finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum AccountType {
    User,
    Organization,
}

#[derive(Debug, Clone, Serialize)]
pub struct Connection {
    pub app_id: i64,
    pub app_slug: String,
    pub app_name: String,
    pub account: String,
    pub account_type: AccountType,
    pub installation_id: Option<i64>,
    pub connected_at: String,
}

#[derive(Debug, Clone)]
pub struct NewConnection {
    pub app_id: i64,
    pub app_slug: String,
    pub app_name: String,
    pub account: String,
    pub account_type: AccountType,
    pub private_key: String,
    pub webhook_secret: String,
    pub client_id: String,
    pub client_secret: String,
}

/// The account half of `owner/repo`, which names the App that can read it.
pub fn owner_of(full_name: &str) -> &str {
    full_name.split('/').next().unwrap_or(full_name)
}

/// Connecting an account again replaces its App; the previous one keeps existing on GitHub.
pub async fn save(state: &State, saved: NewConnection) -> anyhow::Result<Connection> {
    let private_key = secrets::encrypt(&state.key, &saved.private_key);
    let webhook_secret = secrets::encrypt(&state.key, &saved.webhook_secret);
    let client_secret = secrets::encrypt(&state.key, &saved.client_secret);
    let mut tx = state.pool.begin().await?;
    sqlx::query!(
        "DELETE FROM github_apps WHERE account = ? OR app_id = ?",
        saved.account,
        saved.app_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO github_apps
             (app_id, app_slug, app_name, account, account_type, private_key, webhook_secret,
              client_id, client_secret, installation_id, connected_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, datetime('now'))",
        saved.app_id,
        saved.app_slug,
        saved.app_name,
        saved.account,
        saved.account_type,
        private_key,
        webhook_secret,
        saved.client_id,
        client_secret,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    by_app(state, saved.app_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the connection vanished as it was saved"))
}

pub async fn load_all(state: &State) -> anyhow::Result<Vec<Connection>> {
    let rows = sqlx::query!(
        r#"SELECT app_id AS "app_id!", app_slug AS "app_slug!", app_name AS "app_name!",
                  account AS "account!", account_type AS "account_type!: AccountType",
                  installation_id, connected_at AS "connected_at!"
           FROM github_apps ORDER BY connected_at, app_id"#
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Connection {
            app_id: r.app_id,
            app_slug: r.app_slug,
            app_name: r.app_name,
            account: r.account,
            account_type: r.account_type,
            installation_id: r.installation_id,
            connected_at: time::utc(r.connected_at),
        })
        .collect())
}

pub async fn by_account(state: &State, account: &str) -> anyhow::Result<Option<Connection>> {
    Ok(load_all(state)
        .await?
        .into_iter()
        .find(|c| c.account.eq_ignore_ascii_case(account)))
}

pub async fn by_app(state: &State, app_id: i64) -> anyhow::Result<Option<Connection>> {
    Ok(load_all(state)
        .await?
        .into_iter()
        .find(|c| c.app_id == app_id))
}

pub async fn private_key(state: &State, app_id: i64) -> anyhow::Result<Option<String>> {
    let row = sqlx::query!(
        r#"SELECT private_key AS "private_key!" FROM github_apps WHERE app_id = ?"#,
        app_id
    )
    .fetch_optional(&state.pool)
    .await?;
    row.map(|r| secrets::decrypt(&state.key, &r.private_key))
        .transpose()
}

/// Every App's secret with its id, so a delivery can be matched to the App that signed it.
pub async fn webhook_secrets(state: &State) -> anyhow::Result<Vec<(i64, String)>> {
    let rows = sqlx::query!(
        r#"SELECT app_id AS "app_id!", webhook_secret AS "webhook_secret!" FROM github_apps"#
    )
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter()
        .map(|r| Ok((r.app_id, secrets::decrypt(&state.key, &r.webhook_secret)?)))
        .collect()
}

pub async fn set_installation(
    state: &State,
    app_id: i64,
    installation_id: i64,
) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE github_apps SET installation_id = ? WHERE app_id = ?",
        installation_id,
        app_id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn disconnect(state: &State, app_id: i64) -> anyhow::Result<bool> {
    let done = sqlx::query!("DELETE FROM github_apps WHERE app_id = ?", app_id)
        .execute(&state.pool)
        .await?;
    Ok(done.rows_affected() > 0)
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
            account_type: AccountType::User,
            private_key: TEST_PEM.into(),
            webhook_secret: "whsec_test".into(),
            client_id: "Iv1.abc".into(),
            client_secret: "cs_abc".into(),
        }
    }

    pub fn org_sample() -> NewConnection {
        NewConnection {
            app_id: 67890,
            app_slug: "ferrum-acme-panel-example".into(),
            app_name: "ferrum-acme-panel-example".into(),
            account: "acme".into(),
            account_type: AccountType::Organization,
            private_key: TEST_PEM.into(),
            webhook_secret: "whsec_acme".into(),
            client_id: "Iv1.def".into(),
            client_secret: "cs_def".into(),
        }
    }

    #[tokio::test]
    async fn a_connection_round_trips_without_exposing_its_secrets() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();

        let loaded = by_account(&state, "IRIXSOFT").await.unwrap().unwrap();
        assert_eq!(loaded.app_name, "ferrum-panel-example");
        assert_eq!(loaded.account, "irixsoft");
        assert_eq!(loaded.account_type, AccountType::User);
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
            private_key(&state, 12345)
                .await
                .unwrap()
                .unwrap()
                .contains("PRIVATE KEY")
        );
        assert_eq!(
            webhook_secrets(&state).await.unwrap(),
            vec![(12345, "whsec_test".to_string())]
        );
    }

    #[tokio::test]
    async fn each_account_has_one_app_and_reconnecting_replaces_it() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();
        save(&state, org_sample()).await.unwrap();
        let mut again = sample();
        again.app_id = 12346;
        again.app_name = "ferrum-panel-second".into();
        save(&state, again).await.unwrap();

        let all = load_all(&state).await.unwrap();
        let mut names: Vec<(&str, i64)> =
            all.iter().map(|c| (c.account.as_str(), c.app_id)).collect();
        names.sort();
        assert_eq!(names, vec![("acme", 67890), ("irixsoft", 12346)]);
        assert!(by_app(&state, 12345).await.unwrap().is_none());
        assert_eq!(
            by_account(&state, "acme")
                .await
                .unwrap()
                .unwrap()
                .account_type,
            AccountType::Organization
        );
    }

    #[tokio::test]
    async fn reconnecting_forgets_the_previous_installation() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();
        set_installation(&state, 12345, 4242).await.unwrap();

        save(&state, sample()).await.unwrap();
        assert_eq!(
            by_app(&state, 12345)
                .await
                .unwrap()
                .unwrap()
                .installation_id,
            None,
            "an installation id belongs to the app that issued it"
        );
    }

    #[tokio::test]
    async fn disconnecting_one_account_keeps_the_others() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();
        save(&state, org_sample()).await.unwrap();
        assert!(disconnect(&state, 12345).await.unwrap());
        assert!(!disconnect(&state, 12345).await.unwrap());

        assert!(private_key(&state, 12345).await.unwrap().is_none());
        assert_eq!(
            webhook_secrets(&state).await.unwrap(),
            vec![(67890, "whsec_acme".to_string())]
        );
        assert_eq!(load_all(&state).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn the_installation_id_is_recorded_after_the_app_is_installed() {
        let (_d, state) = state().await;
        save(&state, sample()).await.unwrap();
        set_installation(&state, 12345, 4242).await.unwrap();
        assert_eq!(
            by_app(&state, 12345)
                .await
                .unwrap()
                .unwrap()
                .installation_id,
            Some(4242)
        );
    }

    #[test]
    fn the_owner_is_the_account_half_of_the_name() {
        assert_eq!(owner_of("irixsoft/ledger"), "irixsoft");
        assert_eq!(owner_of("acme"), "acme");
    }
}
