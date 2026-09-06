use crate::secret;
use crate::state::State;
use crate::time;
use crate::users::{self, User};

pub const COOKIE: &str = "ferrum_session";
pub const TTL_DAYS: i64 = 30;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub created_at: String,
    pub last_seen: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Device<'a> {
    pub user_agent: Option<&'a str>,
    pub ip: Option<&'a str>,
}

pub async fn issue(state: &State, user_id: &str, device: Device<'_>) -> anyhow::Result<String> {
    let token = secret::generate();
    let id = secret::hash(&token);
    let ttl = format!("+{TTL_DAYS} days");

    sqlx::query!(
        "INSERT INTO sessions (id, user_id, user_agent, ip, expires_at)
         VALUES (?, ?, ?, ?, datetime('now', ?))",
        id,
        user_id,
        device.user_agent,
        device.ip,
        ttl
    )
    .execute(&state.pool)
    .await?;

    Ok(token)
}

pub fn is_current(token: &str, id: &str) -> bool {
    secret::hash(token) == id
}

pub async fn resolve(state: &State, token: &str) -> anyhow::Result<Option<User>> {
    let id = secret::hash(token);
    let row = sqlx::query!(
        "UPDATE sessions SET last_seen = datetime('now')
         WHERE id = ? AND expires_at > datetime('now')
         RETURNING user_id",
        id
    )
    .fetch_optional(&state.pool)
    .await?;

    match row {
        Some(r) => users::by_id(state, &r.user_id).await,
        None => Ok(None),
    }
}

pub async fn revoke(state: &State, id: &str) -> anyhow::Result<()> {
    sqlx::query!("DELETE FROM sessions WHERE id = ?", id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

pub async fn revoke_for(state: &State, user_id: &str, id: &str) -> anyhow::Result<bool> {
    let done = sqlx::query!(
        "DELETE FROM sessions WHERE id = ? AND user_id = ?",
        id,
        user_id
    )
    .execute(&state.pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn revoke_all_for(state: &State, user_id: &str) -> anyhow::Result<()> {
    sqlx::query!("DELETE FROM sessions WHERE user_id = ?", user_id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

pub async fn revoke_by_token(state: &State, token: &str) -> anyhow::Result<()> {
    revoke(state, &secret::hash(token)).await
}

pub async fn list_for(state: &State, user_id: &str) -> anyhow::Result<Vec<Session>> {
    let rows = sqlx::query!(
        "SELECT id, user_id, user_agent, ip, created_at, last_seen
         FROM sessions WHERE user_id = ? AND expires_at > datetime('now')
         ORDER BY created_at",
        user_id
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Session {
            id: r.id,
            user_id: r.user_id,
            user_agent: r.user_agent,
            ip: r.ip,
            created_at: time::utc(r.created_at),
            last_seen: time::utc(r.last_seen),
        })
        .collect())
}

pub async fn sweep(state: &State) -> anyhow::Result<u64> {
    let done = sqlx::query!("DELETE FROM sessions WHERE expires_at <= datetime('now')")
        .execute(&state.pool)
        .await?;
    Ok(done.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn state() -> (tempfile::TempDir, State) {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(dir.path()).await.unwrap();
        (dir, state)
    }

    fn browser(user_agent: &str) -> Device<'_> {
        Device {
            user_agent: Some(user_agent),
            ip: Some("203.0.113.7"),
        }
    }

    #[tokio::test]
    async fn a_session_resolves_to_its_user() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, browser("Firefox")).await.unwrap();
        assert_eq!(
            resolve(&state, &token).await.unwrap().map(|u| u.id),
            Some(user.id)
        );
    }

    #[tokio::test]
    async fn a_revoked_session_stops_resolving() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, Device::default()).await.unwrap();
        let listed = list_for(&state, &user.id).await.unwrap();
        revoke(&state, &listed[0].id).await.unwrap();
        assert!(resolve(&state, &token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn signing_out_revokes_the_presented_cookie() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, Device::default()).await.unwrap();
        revoke_by_token(&state, &token).await.unwrap();
        assert!(resolve(&state, &token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_expired_session_does_not_resolve_and_is_swept() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, Device::default()).await.unwrap();
        sqlx::query("UPDATE sessions SET expires_at = datetime('now', '-1 day')")
            .execute(&state.pool)
            .await
            .unwrap();

        assert!(resolve(&state, &token).await.unwrap().is_none());
        assert_eq!(sweep(&state).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn the_cookie_value_is_never_the_stored_id() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, Device::default()).await.unwrap();
        let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM sessions")
            .fetch_all(&state.pool)
            .await
            .unwrap();
        assert!(
            !ids.contains(&token),
            "a leaked database read must not yield a usable cookie"
        );
    }

    #[tokio::test]
    async fn revoking_every_session_signs_out_all_devices() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let a = issue(&state, &user.id, browser("Firefox")).await.unwrap();
        let b = issue(&state, &user.id, browser("Safari")).await.unwrap();

        revoke_all_for(&state, &user.id).await.unwrap();

        assert!(resolve(&state, &a).await.unwrap().is_none());
        assert!(resolve(&state, &b).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_device_is_recorded_and_read_back() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        issue(&state, &user.id, browser("Firefox")).await.unwrap();

        let listed = list_for(&state, &user.id).await.unwrap();
        assert_eq!(listed[0].user_agent.as_deref(), Some("Firefox"));
        assert_eq!(listed[0].ip.as_deref(), Some("203.0.113.7"));
    }

    #[tokio::test]
    async fn a_session_is_current_only_for_the_cookie_that_issued_it() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let mine = issue(&state, &user.id, Device::default()).await.unwrap();
        let theirs = issue(&state, &user.id, Device::default()).await.unwrap();

        let listed = list_for(&state, &user.id).await.unwrap();
        let matched: Vec<_> = listed.iter().filter(|s| is_current(&mine, &s.id)).collect();
        assert_eq!(matched.len(), 1);
        assert!(!is_current(&theirs, &matched[0].id));
    }

    #[tokio::test]
    async fn a_session_can_only_be_revoked_by_the_user_that_owns_it() {
        let (_d, state) = state().await;
        let mine = users::create(&state, "Saeed").await.unwrap();
        let theirs = users::create(&state, "Someone else").await.unwrap();
        issue(&state, &theirs.id, Device::default()).await.unwrap();
        let target = list_for(&state, &theirs.id).await.unwrap()[0].id.clone();

        assert!(!revoke_for(&state, &mine.id, &target).await.unwrap());
        assert_eq!(list_for(&state, &theirs.id).await.unwrap().len(), 1);
        assert!(revoke_for(&state, &theirs.id, &target).await.unwrap());
        assert!(list_for(&state, &theirs.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_expired_session_is_not_listed() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        issue(&state, &user.id, Device::default()).await.unwrap();
        sqlx::query("UPDATE sessions SET expires_at = datetime('now', '-1 day')")
            .execute(&state.pool)
            .await
            .unwrap();

        assert!(list_for(&state, &user.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn resolving_updates_last_seen() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, Device::default()).await.unwrap();
        sqlx::query("UPDATE sessions SET last_seen = datetime('now', '-1 hour')")
            .execute(&state.pool)
            .await
            .unwrap();
        let stale = list_for(&state, &user.id).await.unwrap()[0]
            .last_seen
            .clone();

        resolve(&state, &token).await.unwrap();

        let fresh = list_for(&state, &user.id).await.unwrap()[0]
            .last_seen
            .clone();
        assert!(fresh > stale, "{fresh} should be later than {stale}");
    }
}
