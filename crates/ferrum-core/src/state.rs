use crate::secrets::{self, Key};
use anyhow::Context;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct State {
    pub pool: SqlitePool,
    pub data_dir: std::path::PathBuf,
    pub key: Arc<Key>,
}

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

impl State {
    pub async fn open(data_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("creating {}", data_dir.display()))?;
        std::fs::set_permissions(data_dir, std::fs::Permissions::from_mode(0o755))?;

        let db_path = data_dir.join("ferrum.db");
        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await?;
        std::fs::set_permissions(&db_path, std::fs::Permissions::from_mode(0o600))?;

        MIGRATOR
            .run(&pool)
            .await
            .context("running schema migrations")?;
        let key = match Key::open(data_dir)? {
            Some(key) => key,
            None if secrets::any_encrypted(&pool).await? => anyhow::bail!(secrets::MISSING_KEY),
            None => Key::create(data_dir)?,
        };
        let state = Self {
            pool,
            data_dir: data_dir.to_path_buf(),
            key: Arc::new(key),
        };
        let sealed = secrets::migrate(&state)
            .await
            .context("encrypting stored secrets")?;
        if sealed > 0 {
            tracing::info!(rows = sealed, "encrypted stored secrets");
        }
        Ok(state)
    }

    pub async fn get_setting(&self, key: &str) -> anyhow::Result<Option<String>> {
        let row = sqlx::query!("SELECT value FROM settings WHERE key = ?", key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.value))
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> anyhow::Result<()> {
        sqlx::query!(
            "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            key,
            value
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
