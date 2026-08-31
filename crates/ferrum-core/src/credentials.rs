use crate::state::State;

#[derive(Debug, Clone)]
pub struct StoredCredential {
    pub id: String,
    pub user_id: String,
    pub label: Option<String>,
    pub credential: String,
    pub counter: i64,
    pub created_at: String,
    pub last_used: Option<String>,
}

pub async fn save(
    state: &State,
    user_id: &str,
    id: &str,
    label: Option<&str>,
    credential: &str,
) -> anyhow::Result<()> {
    sqlx::query!(
        "INSERT INTO credentials (id, user_id, label, credential) VALUES (?, ?, ?, ?)",
        id,
        user_id,
        label,
        credential
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn by_id(state: &State, id: &str) -> anyhow::Result<Option<StoredCredential>> {
    let row = sqlx::query!(
        "SELECT id, user_id, label, credential, counter, created_at, last_used
         FROM credentials WHERE id = ?",
        id
    )
    .fetch_optional(&state.pool)
    .await?;

    Ok(row.map(|r| StoredCredential {
        id: r.id,
        user_id: r.user_id,
        label: r.label,
        credential: r.credential,
        counter: r.counter,
        created_at: r.created_at,
        last_used: r.last_used,
    }))
}

pub async fn for_user(state: &State, user_id: &str) -> anyhow::Result<Vec<StoredCredential>> {
    let rows = sqlx::query!(
        "SELECT id, user_id, label, credential, counter, created_at, last_used
         FROM credentials WHERE user_id = ? ORDER BY created_at",
        user_id
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| StoredCredential {
            id: r.id,
            user_id: r.user_id,
            label: r.label,
            credential: r.credential,
            counter: r.counter,
            created_at: r.created_at,
            last_used: r.last_used,
        })
        .collect())
}

pub async fn count_for_user(state: &State, user_id: &str) -> anyhow::Result<i64> {
    let row = sqlx::query!(
        r#"SELECT count(*) AS "n!: i64" FROM credentials WHERE user_id = ?"#,
        user_id
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(row.n)
}

pub async fn touch(state: &State, id: &str, counter: u32, credential: &str) -> anyhow::Result<()> {
    let counter = i64::from(counter);
    sqlx::query!(
        "UPDATE credentials SET counter = ?, credential = ?, last_used = datetime('now')
         WHERE id = ?",
        counter,
        credential,
        id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::users;

    async fn state() -> (tempfile::TempDir, State) {
        let dir = tempfile::tempdir().unwrap();
        let state = State::open(dir.path()).await.unwrap();
        (dir, state)
    }

    #[tokio::test]
    async fn a_saved_credential_comes_back_for_its_user() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        save(&state, &user.id, "cred-a", Some("laptop"), "{}")
            .await
            .unwrap();

        let found = for_user(&state, &user.id).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "cred-a");
        assert_eq!(found[0].label.as_deref(), Some("laptop"));
        assert_eq!(found[0].counter, 0);
        assert_eq!(count_for_user(&state, &user.id).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn the_same_credential_cannot_be_registered_to_two_accounts() {
        let (_d, state) = state().await;
        let a = users::create(&state, "Saeed").await.unwrap();
        let b = users::create(&state, "Attacker").await.unwrap();
        save(&state, &a.id, "cred-a", None, "{}").await.unwrap();

        assert!(
            save(&state, &b.id, "cred-a", None, "{}").await.is_err(),
            "a credential id must belong to exactly one account"
        );
    }

    #[tokio::test]
    async fn touch_records_the_new_counter_and_last_use() {
        let (_d, state) = state().await;
        let user = users::create(&state, "Saeed").await.unwrap();
        save(&state, &user.id, "cred-a", None, "{}").await.unwrap();

        touch(&state, "cred-a", 7, r#"{"v":2}"#).await.unwrap();

        let found = by_id(&state, "cred-a").await.unwrap().unwrap();
        assert_eq!(found.counter, 7);
        assert_eq!(found.credential, r#"{"v":2}"#);
        assert!(found.last_used.is_some());
    }
}
