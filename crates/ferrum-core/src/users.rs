use crate::state::State;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub handle: String,
    pub name: String,
    pub created_at: String,
}

impl User {
    pub fn handle_uuid(&self) -> Option<Uuid> {
        Uuid::parse_str(&self.handle).ok()
    }
}

pub async fn create(state: &State, name: &str) -> anyhow::Result<User> {
    let id = Uuid::new_v4().to_string();
    let handle = Uuid::new_v4().to_string();
    let row = sqlx::query!(
        r#"INSERT INTO users (id, handle, name) VALUES (?, ?, ?)
           RETURNING id AS "id!", handle AS "handle!", name AS "name!", created_at AS "created_at!""#,
        id,
        handle,
        name
    )
    .fetch_one(&state.pool)
    .await?;

    Ok(User {
        id: row.id,
        handle: row.handle,
        name: row.name,
        created_at: row.created_at,
    })
}

pub async fn by_id(state: &State, id: &str) -> anyhow::Result<Option<User>> {
    let row = sqlx::query!(
        "SELECT id, handle, name, created_at FROM users WHERE id = ?",
        id
    )
    .fetch_optional(&state.pool)
    .await?;

    Ok(row.map(|r| User {
        id: r.id,
        handle: r.handle,
        name: r.name,
        created_at: r.created_at,
    }))
}

pub async fn by_handle(state: &State, handle: &str) -> anyhow::Result<Option<User>> {
    let row = sqlx::query!(
        "SELECT id, handle, name, created_at FROM users WHERE handle = ?",
        handle
    )
    .fetch_optional(&state.pool)
    .await?;

    Ok(row.map(|r| User {
        id: r.id,
        handle: r.handle,
        name: r.name,
        created_at: r.created_at,
    }))
}

pub async fn list(state: &State) -> anyhow::Result<Vec<User>> {
    let rows = sqlx::query!("SELECT id, handle, name, created_at FROM users ORDER BY created_at")
        .fetch_all(&state.pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|r| User {
            id: r.id,
            handle: r.handle,
            name: r.name,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn count(state: &State) -> anyhow::Result<i64> {
    let row = sqlx::query!(r#"SELECT count(*) AS "n!: i64" FROM users"#)
        .fetch_one(&state.pool)
        .await?;
    Ok(row.n)
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
    async fn handles_are_unique_and_opaque() {
        let (_d, state) = state().await;
        let a = create(&state, "Saeed").await.unwrap();
        let b = create(&state, "Saeed").await.unwrap();
        assert_ne!(a.handle, b.handle);
        assert!(
            !a.handle.contains("Saeed"),
            "the handle must not leak the name"
        );
        assert!(a.handle.len() >= 32);
    }

    #[tokio::test]
    async fn a_handle_is_a_uuid_the_authenticator_can_return() {
        let (_d, state) = state().await;
        let a = create(&state, "Saeed").await.unwrap();
        assert!(
            a.handle_uuid().is_some(),
            "webauthn parses the user handle as a uuid, so it must be one"
        );
    }

    #[tokio::test]
    async fn a_user_is_findable_by_handle_and_id() {
        let (_d, state) = state().await;
        let a = create(&state, "Saeed").await.unwrap();
        assert_eq!(
            by_handle(&state, &a.handle).await.unwrap().map(|u| u.id),
            Some(a.id.clone())
        );
        assert_eq!(by_id(&state, &a.id).await.unwrap(), Some(a));
    }

    #[tokio::test]
    async fn an_unknown_lookup_is_none() {
        let (_d, state) = state().await;
        assert!(by_handle(&state, "nobody").await.unwrap().is_none());
        assert!(by_id(&state, "nobody").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn count_and_list_track_creation() {
        let (_d, state) = state().await;
        assert_eq!(count(&state).await.unwrap(), 0);
        create(&state, "Saeed").await.unwrap();
        create(&state, "Teammate").await.unwrap();
        assert_eq!(count(&state).await.unwrap(), 2);
        assert_eq!(list(&state).await.unwrap().len(), 2);
    }
}
