use crate::apps::App;
use crate::postgres::{self, MAINTENANCE_DB, sql};
use crate::state::State;
use crate::{SNAPSHOTS_DIR, time};
use ferrum_platform::Platform;
use ferrum_platform::ubuntu::PG_USER;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const KEEP: usize = 10;

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub id: String,
    pub database_id: String,
    pub database: String,
    pub deploy_id: Option<String>,
    pub path: String,
    pub taken_at: String,
}

pub fn dir(database: &str) -> PathBuf {
    Path::new(SNAPSHOTS_DIR).join(database)
}

pub fn path(database: &str, deploy_id: &str) -> PathBuf {
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    dir(database).join(format!("{stamp}_{}.dump", super::short(deploy_id)))
}

/// One `pg_dump` per database linked to the app.
pub async fn take(
    state: &State,
    platform: &dyn Platform,
    app: &App,
    deploy_id: &str,
) -> anyhow::Result<Vec<Snapshot>> {
    let mut taken = Vec::new();
    for db in postgres::linked_to(state, &app.id).await? {
        let dir = dir(&db.name);
        platform.make_dirs(&dir, 0o750)?;
        platform.chown_tree(&dir, PG_USER)?;
        let path = path(&db.name, deploy_id);
        platform.postgres_dump(&db.name, &path)?;
        let id = uuid::Uuid::new_v4().to_string();
        let path = path.to_string_lossy().to_string();
        sqlx::query!(
            "INSERT INTO snapshots (id, database_id, deploy_id, path) VALUES (?, ?, ?, ?)",
            id,
            db.id,
            deploy_id,
            path
        )
        .execute(&state.pool)
        .await?;
        prune(state, platform, &db.id).await?;
        if let Some(snapshot) = by_id(state, &id).await? {
            taken.push(snapshot);
        }
    }
    Ok(taken)
}

/// The database is dropped and recreated under its role, then the dump is restored as the
/// cluster superuser in one transaction, so ownership comes back exactly as it was dumped.
pub async fn restore(
    state: &State,
    platform: &dyn Platform,
    snapshot_id: &str,
) -> anyhow::Result<()> {
    let snapshot = by_id(state, snapshot_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("No such snapshot."))?;
    let db = postgres::by_id(state, &snapshot.database_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("The database of that snapshot no longer exists."))?;
    platform.postgres_sql(MAINTENANCE_DB, &sql::recreate_database(&db.name, &db.role))?;
    platform.postgres_restore(&db.name, Path::new(&snapshot.path))?;
    Ok(())
}

pub async fn prune(
    state: &State,
    platform: &dyn Platform,
    database_id: &str,
) -> anyhow::Result<()> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", path AS "path!" FROM snapshots WHERE database_id = ? ORDER BY rowid DESC"#,
        database_id
    )
    .fetch_all(&state.pool)
    .await?;
    for row in rows.iter().skip(KEEP) {
        platform.remove_file(Path::new(&row.path))?;
        sqlx::query!("DELETE FROM snapshots WHERE id = ?", row.id)
            .execute(&state.pool)
            .await?;
    }
    Ok(())
}

pub async fn by_id(state: &State, id: &str) -> anyhow::Result<Option<Snapshot>> {
    Ok(fetch(state, Some(id), None).await?.into_iter().next())
}

pub async fn for_deploy(state: &State, deploy_id: &str) -> anyhow::Result<Vec<Snapshot>> {
    fetch(state, None, Some(deploy_id)).await
}

async fn fetch(
    state: &State,
    id: Option<&str>,
    deploy_id: Option<&str>,
) -> anyhow::Result<Vec<Snapshot>> {
    let rows = sqlx::query!(
        r#"SELECT s.id AS "id!", s.database_id AS "database_id!", d.name AS "database!",
                  s.deploy_id, s.path AS "path!", s.taken_at AS "taken_at!"
           FROM snapshots s JOIN databases d ON d.id = s.database_id
           WHERE (? IS NULL OR s.id = ?) AND (? IS NULL OR s.deploy_id = ?)
           ORDER BY s.rowid"#,
        id,
        id,
        deploy_id,
        deploy_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Snapshot {
            id: r.id,
            database_id: r.database_id,
            database: r.database,
            deploy_id: r.deploy_id,
            path: r.path,
            taken_at: time::utc(r.taken_at),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::apps::{self};
    use crate::deploy::{Commit, Trigger, create};
    use ferrum_platform::FakePlatform;

    #[tokio::test]
    async fn a_snapshot_is_dumped_as_postgres_into_its_own_directory_and_pruned_to_ten() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        postgres::create(&state, &p, postgres::tests::new("ledger_prod"))
            .await
            .unwrap();
        postgres::link(&state, &app.id, "ledger_prod")
            .await
            .unwrap();
        let mut last = Vec::new();
        for _ in 0..11 {
            let d = create(&state, &app, Trigger::Manual, "main", &Commit::default())
                .await
                .unwrap();
            last = take(&state, &p, &app, &d.id).await.unwrap();
        }
        assert_eq!(last.len(), 1);
        assert!(
            last[0]
                .path
                .starts_with("/var/lib/ferrum/snapshots/ledger_prod/")
        );
        assert!(last[0].path.ends_with(".dump"));
        assert_eq!(last[0].database, "ledger_prod");
        assert!(last[0].taken_at.ends_with('Z'));
        let calls = p.calls();
        assert!(
            calls
                .contains(&"chown_tree /var/lib/ferrum/snapshots/ledger_prod postgres".to_string())
        );
        assert_eq!(p.calls_matching("postgres_dump ledger_prod").len(), 11);
        assert_eq!(
            p.calls_matching("remove_file /var/lib/ferrum/snapshots")
                .len(),
            1
        );
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM snapshots")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(rows, 10);
    }

    #[tokio::test]
    async fn a_restore_recreates_the_database_under_its_role_and_restores_as_postgres() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        postgres::create(&state, &p, postgres::tests::new("ledger_prod"))
            .await
            .unwrap();
        postgres::link(&state, &app.id, "ledger_prod")
            .await
            .unwrap();
        let d = create(&state, &app, Trigger::Manual, "main", &Commit::default())
            .await
            .unwrap();
        let taken = take(&state, &p, &app, &d.id).await.unwrap();
        restore(&state, &p, &taken[0].id).await.unwrap();
        let sql = p.sql();
        let recreate = sql.last().unwrap();
        assert!(recreate.starts_with("DROP DATABASE IF EXISTS \"ledger_prod\" WITH (FORCE);"));
        assert!(recreate.contains("CREATE DATABASE \"ledger_prod\" OWNER \"ledger_prod\";"));
        assert!(recreate.contains("REVOKE CONNECT ON DATABASE \"ledger_prod\" FROM PUBLIC;"));
        assert!(
            !recreate.contains("DROP ROLE"),
            "the role and its password must survive"
        );
        assert!(
            p.calls()
                .last()
                .unwrap()
                .starts_with("postgres_restore ledger_prod /var/lib/ferrum/snapshots/ledger_prod/")
        );
        assert!(restore(&state, &p, "nope").await.is_err());
    }
}
