use super::Api;
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
    pub async fn repos(&self, state: &State) -> anyhow::Result<Vec<Repo>> {
        let client = self.installed(state).await?;
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

        found.sort_by(|a, b| a.full_name.cmp(&b.full_name));
        Ok(found)
    }
}
