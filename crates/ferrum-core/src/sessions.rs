use crate::secret;
use crate::state::State;
use crate::users::{self, User};

pub const COOKIE: &str = "ferrum_session";
pub const TTL_DAYS: i64 = 30;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub user_agent: Option<String>,
    pub created_at: String,
    pub last_seen: String,
}

pub async fn issue(
    state: &State,
    user_id: &str,
    user_agent: Option<&str>,
) -> anyhow::Result<String> {
    let token = secret::generate();
    let id = secret::hash(&token);
    let ttl = format!("+{TTL_DAYS} days");

    sqlx::query!(
        "INSERT INTO sessions (id, user_id, user_agent, expires_at)
         VALUES (?, ?, ?, datetime('now', ?))",
        id,
        user_id,
        user_agent,
        ttl
    )
    .execute(&state.pool)
    .await?;

    Ok(token)
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
        "SELECT id, user_id, user_agent, created_at, last_seen
         FROM sessions WHERE user_id = ? ORDER BY created_at",
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
            created_at: r.created_at,
            last_seen: r.last_seen,
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

    #[tokio::test]
    async fn a_session_resolves_to_its_user() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, Some("Firefox")).await.unwrap();
        assert_eq!(
            resolve(&state, &token).await.unwrap().map(|u| u.id),
            Some(user.id)
        );
    }

    #[tokio::test]
    async fn a_revoked_session_stops_resolving() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, None).await.unwrap();
        let listed = list_for(&state, &user.id).await.unwrap();
        revoke(&state, &listed[0].id).await.unwrap();
        assert!(resolve(&state, &token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn signing_out_revokes_the_presented_cookie() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, None).await.unwrap();
        revoke_by_token(&state, &token).await.unwrap();
        assert!(resolve(&state, &token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_expired_session_does_not_resolve_and_is_swept() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, None).await.unwrap();
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
        let token = issue(&state, &user.id, None).await.unwrap();
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
        let a = issue(&state, &user.id, Some("Firefox")).await.unwrap();
        let b = issue(&state, &user.id, Some("Safari")).await.unwrap();

        revoke_all_for(&state, &user.id).await.unwrap();

        assert!(resolve(&state, &a).await.unwrap().is_none());
        assert!(resolve(&state, &b).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolving_updates_last_seen() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id, None).await.unwrap();
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
