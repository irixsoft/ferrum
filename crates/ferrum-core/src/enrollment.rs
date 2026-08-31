use crate::secret;
use crate::state::State;
use crate::users::{self, User};
use uuid::Uuid;

pub const TTL_MINUTES: i64 = 60;

pub async fn issue(state: &State, user_id: &str) -> anyhow::Result<String> {
    let token = secret::generate();
    let id = Uuid::new_v4().to_string();
    let hash = secret::hash(&token);
    let ttl = format!("+{TTL_MINUTES} minutes");

    sqlx::query!(
        "INSERT INTO enrollments (id, user_id, hash, expires_at)
         VALUES (?, ?, ?, datetime('now', ?))",
        id,
        user_id,
        hash,
        ttl
    )
    .execute(&state.pool)
    .await?;

    Ok(token)
}

pub async fn redeem(state: &State, token: &str) -> anyhow::Result<Option<User>> {
    let hash = secret::hash(token);
    let row = sqlx::query!(
        "UPDATE enrollments SET used_at = datetime('now')
         WHERE hash = ? AND used_at IS NULL AND expires_at > datetime('now')
         RETURNING user_id",
        hash
    )
    .fetch_optional(&state.pool)
    .await?;

    match row {
        Some(r) => users::by_id(state, &r.user_id).await,
        None => Ok(None),
    }
}

pub fn url(hostname: &str, token: &str) -> String {
    format!("https://{hostname}/enroll/{token}")
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
    async fn a_link_redeems_exactly_once() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id).await.unwrap();

        let first = redeem(&state, &token).await.unwrap();
        assert_eq!(first.map(|u| u.id), Some(user.id.clone()));
        assert!(
            redeem(&state, &token).await.unwrap().is_none(),
            "a used link must not work twice"
        );
    }

    #[tokio::test]
    async fn an_unknown_token_redeems_to_nothing() {
        let (_d, state) = state().await;
        assert!(redeem(&state, "not-a-real-token").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_expired_link_is_refused() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id).await.unwrap();
        sqlx::query("UPDATE enrollments SET expires_at = datetime('now', '-1 minute')")
            .execute(&state.pool)
            .await
            .unwrap();

        assert!(redeem(&state, &token).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_plaintext_token_is_never_stored() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        let token = issue(&state, &user.id).await.unwrap();

        let rows: Vec<String> = sqlx::query_scalar("SELECT hash FROM enrollments")
            .fetch_all(&state.pool)
            .await
            .unwrap();
        assert!(
            !rows.iter().any(|h| h.contains(&token)),
            "the link was stored in the clear"
        );
    }

    #[test]
    fn the_url_is_https_and_carries_the_token() {
        assert_eq!(
            url("panel.example.com", "abc123"),
            "https://panel.example.com/enroll/abc123"
        );
    }
}
