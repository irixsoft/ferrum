use crate::secret;
use crate::state::State;
use uuid::Uuid;

pub const PREFIX: &str = "ferr_";

#[derive(Debug, Clone)]
pub struct ApiToken {
    pub id: String,
    pub name: String,
    pub read_only: bool,
    pub created_at: String,
    pub last_used: Option<String>,
}

pub struct MintedToken {
    pub token: ApiToken,
    pub secret: String,
}

pub async fn mint(state: &State, name: &str, read_only: bool) -> anyhow::Result<MintedToken> {
    let id = Uuid::new_v4().to_string();
    let plaintext = format!("{PREFIX}{}", secret::generate());
    let hash = secret::hash(&plaintext);

    let row = sqlx::query!(
        r#"INSERT INTO api_tokens (id, name, hash, read_only) VALUES (?, ?, ?, ?)
           RETURNING id AS "id!", name AS "name!", created_at AS "created_at!""#,
        id,
        name,
        hash,
        read_only
    )
    .fetch_one(&state.pool)
    .await?;

    Ok(MintedToken {
        token: ApiToken {
            id: row.id,
            name: row.name,
            read_only,
            created_at: row.created_at,
            last_used: None,
        },
        secret: plaintext,
    })
}

pub async fn verify(state: &State, presented: &str) -> anyhow::Result<Option<ApiToken>> {
    if !presented.starts_with(PREFIX) {
        return Ok(None);
    }
    let hash = secret::hash(presented);
    let row = sqlx::query!(
        r#"UPDATE api_tokens SET last_used = datetime('now')
           WHERE hash = ? AND revoked_at IS NULL
           RETURNING id AS "id!", name AS "name!", read_only AS "read_only!: bool",
                     created_at AS "created_at!", last_used"#,
        hash
    )
    .fetch_optional(&state.pool)
    .await?;

    Ok(row.map(|r| ApiToken {
        id: r.id,
        name: r.name,
        read_only: r.read_only,
        created_at: r.created_at,
        last_used: r.last_used,
    }))
}

pub async fn list(state: &State) -> anyhow::Result<Vec<ApiToken>> {
    let rows = sqlx::query!(
        r#"SELECT id, name, read_only AS "read_only: bool", created_at, last_used
           FROM api_tokens WHERE revoked_at IS NULL ORDER BY created_at"#
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ApiToken {
            id: r.id,
            name: r.name,
            read_only: r.read_only,
            created_at: r.created_at,
            last_used: r.last_used,
        })
        .collect())
}

pub async fn revoke(state: &State, id: &str) -> anyhow::Result<()> {
    sqlx::query!(
        "UPDATE api_tokens SET revoked_at = datetime('now') WHERE id = ?",
        id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
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
    async fn a_minted_token_verifies_once_and_keeps_verifying() {
        let (_d, state) = state().await;
        let minted = mint(&state, "my agent", true).await.unwrap();
        assert!(minted.secret.starts_with(PREFIX));
        assert!(verify(&state, &minted.secret).await.unwrap().is_some());
        assert!(verify(&state, &minted.secret).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn the_secret_is_stored_only_as_a_hash() {
        let (_d, state) = state().await;
        let minted = mint(&state, "my agent", false).await.unwrap();
        let hashes: Vec<String> = sqlx::query_scalar("SELECT hash FROM api_tokens")
            .fetch_all(&state.pool)
            .await
            .unwrap();
        assert!(!hashes.iter().any(|h| h.contains(&minted.secret)));
    }

    #[tokio::test]
    async fn a_revoked_token_stops_verifying_and_stops_being_listed() {
        let (_d, state) = state().await;
        let minted = mint(&state, "my agent", false).await.unwrap();
        revoke(&state, &minted.token.id).await.unwrap();
        assert!(verify(&state, &minted.secret).await.unwrap().is_none());
        assert!(list(&state).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_garbage_token_verifies_to_nothing() {
        let (_d, state) = state().await;
        assert!(verify(&state, "ferr_nonsense").await.unwrap().is_none());
        assert!(verify(&state, "").await.unwrap().is_none());
        assert!(verify(&state, "not-even-prefixed").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_read_only_flag_survives_a_round_trip() {
        let (_d, state) = state().await;
        let reader = mint(&state, "reader", true).await.unwrap();
        let writer = mint(&state, "writer", false).await.unwrap();
        assert!(
            verify(&state, &reader.secret)
                .await
                .unwrap()
                .unwrap()
                .read_only
        );
        assert!(
            !verify(&state, &writer.secret)
                .await
                .unwrap()
                .unwrap()
                .read_only
        );
        assert!(list(&state).await.unwrap().iter().any(|t| t.read_only));
    }

    #[tokio::test]
    async fn verifying_records_last_use() {
        let (_d, state) = state().await;
        let minted = mint(&state, "my agent", false).await.unwrap();
        assert!(minted.token.last_used.is_none());
        assert!(
            verify(&state, &minted.secret)
                .await
                .unwrap()
                .unwrap()
                .last_used
                .is_some()
        );
    }
}
