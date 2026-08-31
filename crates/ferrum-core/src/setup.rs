use crate::state::State;

const STAGE_SETTING: &str = "setup.stage";
const HOSTNAME_SETTING: &str = "setup.hostname";
const EMAIL_SETTING: &str = "setup.email";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Fresh,
    HostnameSet,
    PlatformInstalled,
    CertIssued,
    Complete,
}

impl Stage {
    pub fn rank(self) -> i64 {
        match self {
            Self::Fresh => 0,
            Self::HostnameSet => 1,
            Self::PlatformInstalled => 2,
            Self::CertIssued => 3,
            Self::Complete => 4,
        }
    }

    pub fn from_rank(n: i64) -> Self {
        match n {
            1 => Self::HostnameSet,
            2 => Self::PlatformInstalled,
            3 => Self::CertIssued,
            n if n >= 4 => Self::Complete,
            _ => Self::Fresh,
        }
    }
}

pub async fn stage(state: &State) -> anyhow::Result<Stage> {
    Ok(state
        .get_setting(STAGE_SETTING)
        .await?
        .and_then(|v| v.parse().ok())
        .map(Stage::from_rank)
        .unwrap_or(Stage::Fresh))
}

pub async fn advance(state: &State, to: Stage) -> anyhow::Result<()> {
    if stage(state).await? >= to {
        return Ok(());
    }
    state
        .set_setting(STAGE_SETTING, &to.rank().to_string())
        .await
}

pub async fn hostname(state: &State) -> anyhow::Result<Option<String>> {
    state.get_setting(HOSTNAME_SETTING).await
}

pub async fn set_hostname(state: &State, host: &str) -> anyhow::Result<()> {
    state.set_setting(HOSTNAME_SETTING, host).await
}

pub async fn email(state: &State) -> anyhow::Result<Option<String>> {
    state.get_setting(EMAIL_SETTING).await
}

pub async fn set_email(state: &State, email: &str) -> anyhow::Result<()> {
    state.set_setting(EMAIL_SETTING, email).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn state() -> (tempfile::TempDir, State) {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(dir.path()).await.unwrap();
        (dir, state)
    }

    #[tokio::test]
    async fn setup_stage_starts_fresh_and_advances_monotonically() {
        let (_dir, state) = state().await;
        assert_eq!(stage(&state).await.unwrap(), Stage::Fresh);
        advance(&state, Stage::HostnameSet).await.unwrap();
        assert_eq!(stage(&state).await.unwrap(), Stage::HostnameSet);
    }

    #[tokio::test]
    async fn advance_never_moves_backwards() {
        let (_dir, state) = state().await;
        advance(&state, Stage::CertIssued).await.unwrap();
        advance(&state, Stage::Fresh).await.unwrap();
        assert_eq!(stage(&state).await.unwrap(), Stage::CertIssued);
    }

    #[tokio::test]
    async fn hostname_and_email_round_trip() {
        let (_dir, state) = state().await;
        assert!(hostname(&state).await.unwrap().is_none());
        set_hostname(&state, "panel.example.com").await.unwrap();
        set_email(&state, "me@example.com").await.unwrap();
        assert_eq!(
            hostname(&state).await.unwrap().as_deref(),
            Some("panel.example.com")
        );
        assert_eq!(
            email(&state).await.unwrap().as_deref(),
            Some("me@example.com")
        );
    }

    #[test]
    fn stage_ranks_round_trip() {
        for s in [
            Stage::Fresh,
            Stage::HostnameSet,
            Stage::PlatformInstalled,
            Stage::CertIssued,
            Stage::Complete,
        ] {
            assert_eq!(Stage::from_rank(s.rank()), s);
        }
    }

    #[test]
    fn an_unknown_future_rank_reads_as_complete() {
        assert_eq!(Stage::from_rank(99), Stage::Complete);
    }
}
