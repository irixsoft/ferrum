use anyhow::{Context, bail};
use ferrum_core::state::State;
use ferrum_core::{enrollment, setup, tokens, users};
use std::path::Path;

pub async fn enrollment_link(data_dir: &Path, wanted: Option<&str>) -> anyhow::Result<String> {
    let state = State::open(data_dir).await?;
    let hostname = setup::hostname(&state)
        .await?
        .context("Ferrum is not set up yet. Run `ferrum setup` first.")?;

    let user = pick(&state, wanted).await?;
    let token = enrollment::issue(&state, &user.id).await?;
    Ok(enrollment::url(&hostname, &token))
}

pub async fn mint_token(data_dir: &Path, name: &str, read_only: bool) -> anyhow::Result<String> {
    let state = State::open(data_dir).await?;
    let minted = tokens::mint(&state, name, read_only).await?;
    Ok(minted.secret)
}

async fn pick(state: &State, wanted: Option<&str>) -> anyhow::Result<users::User> {
    let all = users::list(state).await?;
    if all.is_empty() {
        bail!("There are no users yet. Run `ferrum setup` to create the first one.");
    }

    let Some(wanted) = wanted else {
        if all.len() == 1 {
            return Ok(all.into_iter().next().expect("checked non-empty"));
        }
        bail!(
            "This panel has {} users, so name one: --user <name>. Known: {}",
            all.len(),
            names(&all)
        );
    };

    let matched: Vec<users::User> = all
        .iter()
        .filter(|u| u.name.eq_ignore_ascii_case(wanted))
        .cloned()
        .collect();

    match matched.len() {
        1 => Ok(matched.into_iter().next().expect("checked length")),
        0 => bail!("No user called \"{wanted}\". Known: {}", names(&all)),
        n => bail!("{n} users are called \"{wanted}\"; rename one in the panel first."),
    }
}

fn names(all: &[users::User]) -> String {
    all.iter()
        .map(|u| u.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn ready() -> (tempfile::TempDir, State) {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(dir.path()).await.unwrap();
        setup::set_hostname(&state, "panel.example.com")
            .await
            .unwrap();
        (dir, state)
    }

    #[tokio::test]
    async fn a_link_is_printed_for_the_only_user() {
        let (dir, state) = ready().await;
        users::create(&state, "Saeed").await.unwrap();

        let url = enrollment_link(dir.path(), None).await.unwrap();
        assert!(
            url.starts_with("https://panel.example.com/enroll/"),
            "{url}"
        );
    }

    #[tokio::test]
    async fn several_users_need_naming() {
        let (dir, state) = ready().await;
        users::create(&state, "Saeed").await.unwrap();
        users::create(&state, "Teammate").await.unwrap();

        let e = enrollment_link(dir.path(), None).await.unwrap_err();
        assert!(e.to_string().contains("--user"), "{e}");

        assert!(
            enrollment_link(dir.path(), Some("teammate")).await.is_ok(),
            "the name match should ignore case"
        );
    }

    #[tokio::test]
    async fn an_unknown_name_lists_the_known_ones() {
        let (dir, state) = ready().await;
        users::create(&state, "Saeed").await.unwrap();

        let e = enrollment_link(dir.path(), Some("nobody"))
            .await
            .unwrap_err();
        assert!(e.to_string().contains("Saeed"), "{e}");
    }

    #[tokio::test]
    async fn enrolling_before_setup_says_so() {
        let dir = tempfile::tempdir().unwrap();
        State::open(dir.path()).await.unwrap();

        let e = enrollment_link(dir.path(), None).await.unwrap_err();
        assert!(e.to_string().contains("ferrum setup"), "{e}");
    }

    #[tokio::test]
    async fn a_minted_token_is_prefixed_and_verifies() {
        let (dir, state) = ready().await;
        let secret = mint_token(dir.path(), "agent", true).await.unwrap();

        assert!(secret.starts_with("ferr_"));
        let verified = tokens::verify(&state, &secret).await.unwrap().unwrap();
        assert!(verified.read_only);
    }
}
