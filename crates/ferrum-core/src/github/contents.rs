use super::Api;
use crate::state::State;
use anyhow::Context;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Listing {
    pub paths: Vec<String>,
    pub truncated: bool,
}

#[derive(Deserialize)]
struct TreeResponse {
    tree: Vec<TreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Deserialize)]
struct TreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct Contents {
    #[serde(rename = "type")]
    kind: String,
    encoding: Option<String>,
    content: Option<String>,
}

fn not_found(e: &octocrab::Error) -> bool {
    matches!(e, octocrab::Error::GitHub { source, .. } if source.status_code.as_u16() == 404)
}

impl Api {
    pub async fn tree(
        &self,
        state: &State,
        full_name: &str,
        git_ref: &str,
    ) -> anyhow::Result<Listing> {
        let client = self.installed(state).await?;
        let route = format!("/repos/{full_name}/git/trees/{git_ref}?recursive=1");
        let response: TreeResponse = match client.get(&route, None::<&()>).await {
            Ok(r) => r,
            Err(e) if not_found(&e) => {
                anyhow::bail!("GitHub has no branch or tag named {git_ref} in {full_name}.")
            }
            Err(e) => return Err(e).context("listing the repository tree"),
        };
        Ok(Listing {
            paths: response
                .tree
                .into_iter()
                .filter(|e| e.kind == "blob")
                .map(|e| e.path)
                .collect(),
            truncated: response.truncated,
        })
    }

    /// `None` when the path is absent, is not a file, or is over GitHub's 1 MB contents limit.
    pub async fn file(
        &self,
        state: &State,
        full_name: &str,
        git_ref: &str,
        path: &str,
    ) -> anyhow::Result<Option<String>> {
        let client = self.installed(state).await?;
        let route = format!("/repos/{full_name}/contents/{path}?ref={git_ref}");
        let found: Contents = match client.get(&route, None::<&()>).await {
            Ok(c) => c,
            Err(e) if not_found(&e) => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {path} from {full_name}")),
        };
        if found.kind != "file" || found.encoding.as_deref() != Some("base64") {
            return Ok(None);
        }
        let packed: String = found
            .content
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let bytes = STANDARD
            .decode(packed)
            .with_context(|| format!("{path} did not decode as base64"))?;
        Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
    }
}
