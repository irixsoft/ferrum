use ferrum_core::state::State;
use std::os::unix::fs::PermissionsExt;

async fn temp_state() -> (tempfile::TempDir, State) {
    let dir = tempfile::tempdir().unwrap();
    let state = State::open(dir.path()).await.unwrap();
    (dir, state)
}

#[tokio::test]
async fn open_creates_database_with_owner_only_permissions() {
    let (dir, _state) = temp_state().await;
    let db = dir.path().join("ferrum.db");
    assert!(db.exists());
    let mode = std::fs::metadata(&db).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "database must not be readable by other users");
}

#[tokio::test]
async fn settings_round_trip() {
    let (_dir, state) = temp_state().await;
    assert_eq!(state.get_setting("hostname").await.unwrap(), None);
    state
        .set_setting("hostname", "panel.example.com")
        .await
        .unwrap();
    assert_eq!(
        state.get_setting("hostname").await.unwrap(),
        Some("panel.example.com".to_string())
    );
}

#[tokio::test]
async fn set_setting_overwrites() {
    let (_dir, state) = temp_state().await;
    state.set_setting("k", "one").await.unwrap();
    state.set_setting("k", "two").await.unwrap();
    assert_eq!(
        state.get_setting("k").await.unwrap(),
        Some("two".to_string())
    );
}

#[tokio::test]
async fn identity_tables_exist_after_migration() {
    let (_dir, state) = temp_state().await;
    for table in [
        "users",
        "credentials",
        "sessions",
        "api_tokens",
        "enrollments",
    ] {
        let found: Option<String> =
            sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name = ?")
                .bind(table)
                .fetch_optional(&state.pool)
                .await
                .unwrap();
        assert_eq!(found.as_deref(), Some(table), "missing table {table}");
    }
}

#[tokio::test]
async fn deleting_a_user_takes_their_credentials_and_sessions() {
    let (_dir, state) = temp_state().await;
    sqlx::query("INSERT INTO users (id, handle, name) VALUES ('u1', 'h1', 'Saeed')")
        .execute(&state.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO credentials (id, user_id, credential) VALUES ('c1', 'u1', '{}')")
        .execute(&state.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sessions (id, user_id, expires_at) VALUES ('s1', 'u1', datetime('now', '+1 day'))",
    )
    .execute(&state.pool)
    .await
    .unwrap();

    sqlx::query("DELETE FROM users WHERE id = 'u1'")
        .execute(&state.pool)
        .await
        .unwrap();

    for table in ["credentials", "sessions"] {
        let left: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(left, 0, "{table} must not outlive their user");
    }
}

#[tokio::test]
async fn open_is_idempotent_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    {
        let state = State::open(dir.path()).await.unwrap();
        state.set_setting("k", "kept").await.unwrap();
    }
    let state = State::open(dir.path()).await.unwrap();
    assert_eq!(
        state.get_setting("k").await.unwrap(),
        Some("kept".to_string())
    );
}
