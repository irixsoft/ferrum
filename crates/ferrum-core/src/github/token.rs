use super::Api;
use crate::state::State;
use anyhow::Context;
use octocrab::Octocrab;
use octocrab::models::{AppId, InstallationId};
use serde::Deserialize;
use std::sync::Arc;

pub const REFRESH_MARGIN_MINUTES: i64 = 5;

pub const NOT_CONNECTED: &str = "GitHub is not connected. Connect it from Settings.";
pub const NOT_INSTALLED: &str = "The GitHub App is not installed on any repository yet.";

pub(super) struct Installed {
    pub app_id: i64,
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
    pub async fn as_app(&self, state: &State) -> anyhow::Result<Octocrab> {
        let connection = super::load(state).await?.context(NOT_CONNECTED)?;
        let pem = super::private_key(state).await?.context(NOT_CONNECTED)?;
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes())
            .context("the stored github app private key is not a valid RSA PEM")?;

        Ok(Octocrab::builder()
            .base_uri(self.base.as_str())?
            .app(AppId(connection.app_id as u64), key)
            .build()?)
    }

    pub async fn installation_id(&self, state: &State) -> anyhow::Result<i64> {
        let connection = super::load(state).await?.context(NOT_CONNECTED)?;
        if let Some(id) = connection.installation_id {
            return Ok(id);
        }

        let found: Vec<InstallationRef> = self
            .as_app(state)
            .await?
            .get("/app/installations", None::<&()>)
            .await
            .context("asking github which accounts the app is installed on")?;

        let id = found.first().context(NOT_INSTALLED)?.id;
        super::set_installation(state, id).await?;
        Ok(id)
    }

    pub(crate) async fn installed(&self, state: &State) -> anyhow::Result<Arc<Octocrab>> {
        let connection = super::load(state).await?.context(NOT_CONNECTED)?;
        let installation_id = self.installation_id(state).await?;

        if let Some(cached) = self.cached(connection.app_id, installation_id) {
            return Ok(cached);
        }

        let client = Arc::new(
            self.as_app(state)
                .await?
                .installation(InstallationId(installation_id as u64))?,
        );
        *self
            .installation
            .lock()
            .expect("the cache lock is not poisoned") = Some(Installed {
            app_id: connection.app_id,
            installation_id,
            client: client.clone(),
        });
        Ok(client)
    }

    fn cached(&self, app_id: i64, installation_id: i64) -> Option<Arc<Octocrab>> {
        let guard = self
            .installation
            .lock()
            .expect("the cache lock is not poisoned");
        let installed = guard.as_ref()?;
        (installed.app_id == app_id && installed.installation_id == installation_id)
            .then(|| installed.client.clone())
    }

    pub async fn installation_token(&self, state: &State) -> anyhow::Result<String> {
        use secrecy::ExposeSecret;

        if let Some(token) = &self.fixed_token {
            return Ok(token.clone());
        }
        let client = self.installed(state).await?;
        let token = client
            .installation_token_with_buffer(chrono::Duration::minutes(REFRESH_MARGIN_MINUTES))
            .await
            .context("minting a github installation token")?;
        Ok(token.expose_secret().to_string())
    }
}
