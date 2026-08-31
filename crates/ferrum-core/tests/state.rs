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
    assert_eq!(state.get_setting("k").await.unwrap(), Some("two".to_string()));
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
