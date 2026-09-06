use super::Api;
use super::token::NOT_INSTALLED;
use crate::state::State;
use anyhow::Context;
use serde::{Deserialize, Serialize};

const PER_PAGE: usize = 100;
const MAX_PAGES: usize = 50;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repo {
    pub full_name: String,
    pub private: bool,
    pub default_branch: String,
    pub pushed_at: Option<String>,
}

#[derive(Deserialize)]
struct Page {
    total_count: usize,
    repositories: Vec<Repo>,
}

impl Api {
    /// Everything every connected App can read. An App with no installation yet contributes
    /// nothing; only when none is installed is that the answer.
    pub async fn repos(&self, state: &State) -> anyhow::Result<Vec<Repo>> {
        let connections = super::load_all(state).await?;
        if connections.is_empty() {
            anyhow::bail!(super::token::NOT_CONNECTED);
        }
        let mut found: Vec<Repo> = Vec::new();
        let mut installed_somewhere = false;
        for connection in &connections {
            match self.repos_of(state, &connection.account).await {
                Ok(repos) => {
                    installed_somewhere = true;
                    found.extend(repos);
                }
                Err(e) if e.to_string() == NOT_INSTALLED => {}
                Err(e) => return Err(e),
            }
        }
        if !installed_somewhere {
            anyhow::bail!(NOT_INSTALLED);
        }
        found.sort_by(|a, b| a.full_name.cmp(&b.full_name));
        Ok(found)
    }

    pub async fn repos_of(&self, state: &State, owner: &str) -> anyhow::Result<Vec<Repo>> {
        let client = self.installed(state, owner).await?;
        let mut found: Vec<Repo> = Vec::new();

        for page in 1..=MAX_PAGES {
            let route = format!("/installation/repositories?per_page={PER_PAGE}&page={page}");
            let batch: Page = client
                .get(&route, None::<&()>)
                .await
                .context("listing the repositories the app can see")?;

            let empty = batch.repositories.is_empty();
            found.extend(batch.repositories);
            if empty || found.len() >= batch.total_count {
                break;
            }
        }
        Ok(found)
    }
}
