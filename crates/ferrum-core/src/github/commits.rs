use super::Api;
use crate::apps::App;
use crate::state::State;
use anyhow::Context;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    pub git_ref: String,
    pub sha: String,
    pub message: String,
    pub author: String,
}

#[derive(Deserialize)]
struct CommitRef {
    sha: String,
    commit: CommitDetail,
    author: Option<Login>,
}

#[derive(Deserialize)]
struct CommitDetail {
    message: String,
    author: Option<Signature>,
}

#[derive(Deserialize)]
struct Signature {
    name: String,
}

#[derive(Deserialize)]
struct Login {
    login: String,
}

pub async fn commit(
    api: &Api,
    state: &State,
    repository: &str,
    git_ref: &str,
) -> anyhow::Result<Head> {
    let found: CommitRef = api
        .installed(state, super::owner_of(repository))
        .await?
        .get(
            format!("/repos/{repository}/commits/{git_ref}"),
            None::<&()>,
        )
        .await
        .with_context(|| {
            format!("GitHub has no branch, tag or commit {git_ref} in {repository}")
        })?;
    Ok(Head {
        git_ref: git_ref.to_string(),
        sha: found.sha,
        message: found
            .commit
            .message
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
        author: found
            .author
            .map(|a| a.login)
            .or(found.commit.author.map(|a| a.name))
            .unwrap_or_default(),
    })
}

/// What a deploy of the app would build right now: its tag, or an explicit ref once.
pub async fn head_of(
    api: &Api,
    state: &State,
    app: &App,
    git_ref: Option<&str>,
) -> anyhow::Result<Head> {
    commit(api, state, &app.repository, git_ref.unwrap_or(&app.git_ref)).await
}
