use super::{Api, Connection};
use crate::state::State;
use anyhow::Context;
use octocrab::Octocrab;
use octocrab::models::{AppId, InstallationId};
use serde::Deserialize;
use std::sync::Arc;

pub const REFRESH_MARGIN_MINUTES: i64 = 5;

pub const NOT_CONNECTED: &str = "GitHub is not connected. Connect it from Settings.";
pub const NOT_INSTALLED: &str = "The GitHub App is not installed on any repository yet.";
const NO_APP_FOR: &str = "No GitHub App is connected for";

pub fn not_connected_for(owner: &str) -> String {
    format!("{NO_APP_FOR} {owner}. Connect it from Settings.")
}

/// Whether an error is one the user fixes from Settings rather than a fault.
pub fn user_fixable(message: &str) -> bool {
    message == NOT_CONNECTED || message == NOT_INSTALLED || message.starts_with(NO_APP_FOR)
}

pub(super) struct Installed {
    pub installation_id: i64,
    /// Shared, never cloned: `Octocrab`'s clone deep-copies its token cache, so a clone would
    /// re-mint on every call.
    pub client: Arc<Octocrab>,
}

#[derive(Deserialize)]
struct InstallationRef {
    id: i64,
}

impl Api {
    async fn connection_for(&self, state: &State, owner: &str) -> anyhow::Result<Connection> {
        let all = super::load_all(state).await?;
        if all.is_empty() {
            anyhow::bail!(NOT_CONNECTED);
        }
        all.into_iter()
            .find(|c| c.account.eq_ignore_ascii_case(owner))
            .ok_or_else(|| anyhow::anyhow!(not_connected_for(owner)))
    }

    pub async fn as_app(&self, state: &State, connection: &Connection) -> anyhow::Result<Octocrab> {
        let pem = super::private_key(state, connection.app_id)
            .await?
            .context(NOT_CONNECTED)?;
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes())
            .context("the stored github app private key is not a valid RSA PEM")?;

        Ok(Octocrab::builder()
            .base_uri(self.base.as_str())?
            .app(AppId(connection.app_id as u64), key)
            .build()?)
    }

    /// A private App can only be installed on the account that owns it, so its one
    /// installation is discovered once and written down.
    pub async fn installation_id(&self, state: &State, owner: &str) -> anyhow::Result<i64> {
        let connection = self.connection_for(state, owner).await?;
        if let Some(id) = connection.installation_id {
            return Ok(id);
        }

        let found: Vec<InstallationRef> = self
            .as_app(state, &connection)
            .await?
            .get("/app/installations", None::<&()>)
            .await
            .context("asking github which accounts the app is installed on")?;

        let id = found.first().context(NOT_INSTALLED)?.id;
        super::set_installation(state, connection.app_id, id).await?;
        Ok(id)
    }

    pub(crate) async fn installed(
        &self,
        state: &State,
        owner: &str,
    ) -> anyhow::Result<Arc<Octocrab>> {
        let connection = self.connection_for(state, owner).await?;
        let installation_id = self.installation_id(state, owner).await?;

        if let Some(cached) = self.cached(connection.app_id, installation_id) {
            return Ok(cached);
        }

        let client = Arc::new(
            self.as_app(state, &connection)
                .await?
                .installation(InstallationId(installation_id as u64))?,
        );
        self.installations
            .lock()
            .expect("the cache lock is not poisoned")
            .insert(
                connection.app_id,
                Installed {
                    installation_id,
                    client: client.clone(),
                },
            );
        Ok(client)
    }

    fn cached(&self, app_id: i64, installation_id: i64) -> Option<Arc<Octocrab>> {
        let guard = self
            .installations
            .lock()
            .expect("the cache lock is not poisoned");
        let installed = guard.get(&app_id)?;
        (installed.installation_id == installation_id).then(|| installed.client.clone())
    }

    pub async fn installation_token(&self, state: &State, owner: &str) -> anyhow::Result<String> {
        use secrecy::ExposeSecret;

        if let Some(token) = &self.fixed_token {
            return Ok(token.clone());
        }
        let client = self.installed(state, owner).await?;
        let token = client
            .installation_token_with_buffer(chrono::Duration::minutes(REFRESH_MARGIN_MINUTES))
            .await
            .context("minting a github installation token")?;
        Ok(token.expose_secret().to_string())
    }
}
