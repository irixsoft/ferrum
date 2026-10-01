use crate::state::State;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    DeployRefused,
    DeployFailed,
    DeployLive,
    BrokeOnItsOwn,
    UpdateAvailable,
    PackageDropped,
    PortRemoved,
    RoleKept,
    ProcessesChanged,
}

impl Kind {
    pub const ALL: [Kind; 9] = [
        Kind::DeployRefused,
        Kind::DeployFailed,
        Kind::DeployLive,
        Kind::BrokeOnItsOwn,
        Kind::UpdateAvailable,
        Kind::PackageDropped,
        Kind::PortRemoved,
        Kind::RoleKept,
        Kind::ProcessesChanged,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::DeployRefused => "deploy_refused",
            Kind::DeployFailed => "deploy_failed",
            Kind::DeployLive => "deploy_live",
            Kind::BrokeOnItsOwn => "broke_on_its_own",
            Kind::UpdateAvailable => "update_available",
            Kind::PackageDropped => "package_dropped",
            Kind::PortRemoved => "port_removed",
            Kind::RoleKept => "role_kept",
            Kind::ProcessesChanged => "processes_changed",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The four switches under Settings > Notifications; `None` never pushes.
    pub fn pref(self) -> Option<&'static str> {
        match self {
            Kind::DeployRefused | Kind::DeployFailed => Some("deploy_failed"),
            Kind::DeployLive => Some("deploy_live"),
            Kind::BrokeOnItsOwn => Some("broke"),
            Kind::UpdateAvailable => Some("update"),
            _ => None,
        }
    }

    pub fn pushes(self) -> bool {
        self.pref().is_some()
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::DeployRefused => "Deploy refused",
            Kind::DeployFailed => "Deploy failed",
            Kind::DeployLive => "Deploy live",
            Kind::BrokeOnItsOwn => "Something broke",
            Kind::UpdateAvailable => "Update available",
            Kind::PackageDropped => "Package dropped from the Aptfile",
            Kind::PortRemoved => "Route removed",
            Kind::RoleKept => "Role kept",
            Kind::ProcessesChanged => "Processes changed",
        }
    }
}

pub const PREFS: [&str; 4] = ["deploy_failed", "deploy_live", "broke", "update"];

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub id: String,
    pub kind: String,
    pub app_id: Option<String>,
    pub subject: String,
    pub sentence: String,
    pub link: Option<String>,
    pub created_at: String,
    pub read_at: Option<String>,
}

/// Records the event and hands it to the push fan-out when one is attached. A notice kind
/// (one that never pushes) is not repeated while an unread one with the same subject exists.
/// Never fails the caller: a storage error is logged and `None` comes back.
pub async fn emit(
    state: &State,
    kind: Kind,
    app_id: Option<&str>,
    subject: &str,
    sentence: &str,
    link: Option<&str>,
) -> Option<Event> {
    match insert(state, kind, app_id, subject, sentence, link).await {
        Ok(Some(event)) => {
            if kind.pushes()
                && let Some(tx) = &state.events_tx
            {
                let _ = tx.send(event.clone());
            }
            Some(event)
        }
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(kind = kind.as_str(), %error, "event not recorded");
            None
        }
    }
}

async fn insert(
    state: &State,
    kind: Kind,
    app_id: Option<&str>,
    subject: &str,
    sentence: &str,
    link: Option<&str>,
) -> anyhow::Result<Option<Event>> {
    let kind_str = kind.as_str();
    if !kind.pushes() {
        let dup = sqlx::query_scalar!(
            "SELECT count(*) FROM events WHERE kind = ? AND app_id IS ? AND subject = ? AND read_at IS NULL",
            kind_str,
            app_id,
            subject
        )
        .fetch_one(&state.pool)
        .await?;
        if dup > 0 {
            return Ok(None);
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query!(
        "INSERT INTO events (id, kind, app_id, subject, sentence, link) VALUES (?, ?, ?, ?, ?, ?)",
        id,
        kind_str,
        app_id,
        subject,
        sentence,
        link
    )
    .execute(&state.pool)
    .await?;
    by_id(state, &id).await
}

pub async fn by_id(state: &State, id: &str) -> anyhow::Result<Option<Event>> {
    let row = sqlx::query_as!(
        Event,
        r#"SELECT id, kind, app_id, subject, sentence, link, created_at, read_at
           FROM events WHERE id = ?"#,
        id
    )
    .fetch_optional(&state.pool)
    .await?;
    Ok(row)
}

pub async fn list(state: &State, limit: i64, unread_only: bool) -> anyhow::Result<Vec<Event>> {
    let rows = if unread_only {
        sqlx::query_as!(
            Event,
            r#"SELECT id, kind, app_id, subject, sentence, link, created_at, read_at
               FROM events WHERE read_at IS NULL ORDER BY created_at DESC, id DESC LIMIT ?"#,
            limit
        )
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as!(
            Event,
            r#"SELECT id, kind, app_id, subject, sentence, link, created_at, read_at
               FROM events ORDER BY created_at DESC, id DESC LIMIT ?"#,
            limit
        )
        .fetch_all(&state.pool)
        .await?
    };
    Ok(rows)
}

pub async fn unread_count(state: &State) -> anyhow::Result<i64> {
    let n = sqlx::query_scalar!("SELECT count(*) FROM events WHERE read_at IS NULL")
        .fetch_one(&state.pool)
        .await?;
    Ok(n as i64)
}

pub async fn mark_read(state: &State, ids: &[String]) -> anyhow::Result<()> {
    for id in ids {
        sqlx::query!(
            "UPDATE events SET read_at = datetime('now') WHERE id = ? AND read_at IS NULL",
            id
        )
        .execute(&state.pool)
        .await?;
    }
    Ok(())
}

pub async fn mark_all_read(state: &State) -> anyhow::Result<()> {
    sqlx::query!("UPDATE events SET read_at = datetime('now') WHERE read_at IS NULL")
        .execute(&state.pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn state() -> State {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(dir.path()).await.unwrap();
        std::mem::forget(dir);
        state
    }

    #[tokio::test]
    async fn a_notice_is_not_repeated_while_an_unread_one_with_the_same_subject_exists() {
        let s = state().await;
        let first = emit(&s, Kind::PackageDropped, None, "ffmpeg", "dropped", None).await;
        assert!(first.is_some());
        let again = emit(&s, Kind::PackageDropped, None, "ffmpeg", "dropped", None).await;
        assert!(again.is_none());
        assert_eq!(unread_count(&s).await.unwrap(), 1);
        mark_all_read(&s).await.unwrap();
        assert!(
            emit(&s, Kind::PackageDropped, None, "ffmpeg", "dropped", None)
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_push_kind_is_always_recorded_and_reaches_the_channel() {
        let mut s = state().await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        s.events_tx = Some(tx);
        emit(
            &s,
            Kind::DeployLive,
            None,
            "ledger",
            "Live at abc123",
            Some("/apps/ledger"),
        )
        .await;
        emit(
            &s,
            Kind::DeployLive,
            None,
            "ledger",
            "Live at abc123",
            Some("/apps/ledger"),
        )
        .await;
        assert_eq!(unread_count(&s).await.unwrap(), 2);
        assert_eq!(rx.recv().await.unwrap().sentence, "Live at abc123");
        let listed = list(&s, 10, true).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].kind, "deploy_live");
        mark_read(&s, &[listed[0].id.clone()]).await.unwrap();
        assert_eq!(unread_count(&s).await.unwrap(), 1);
    }

    #[test]
    fn only_the_four_chosen_kinds_push_and_every_kind_round_trips() {
        for kind in Kind::ALL {
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
            assert_eq!(kind.pushes(), kind.pref().is_some());
        }
        assert!(Kind::DeployRefused.pushes());
        assert!(Kind::DeployFailed.pushes());
        assert!(Kind::DeployLive.pushes());
        assert!(Kind::BrokeOnItsOwn.pushes());
        assert!(Kind::UpdateAvailable.pushes());
        assert!(!Kind::PackageDropped.pushes());
        assert!(!Kind::PortRemoved.pushes());
        assert!(!Kind::RoleKept.pushes());
        assert!(!Kind::ProcessesChanged.pushes());
        for pref in PREFS {
            assert!(Kind::ALL.iter().any(|k| k.pref() == Some(pref)));
        }
    }
}
