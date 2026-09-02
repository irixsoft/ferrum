pub mod install;
pub mod sql;
pub mod tune;

pub use install::{DEFAULT_MAJOR, ensure_installed, major};

use crate::secret;
use crate::state::State;
use crate::time;
use ferrum_platform::ubuntu::PG_PORT;
use ferrum_platform::{Platform, PlatformError};
use serde::{Deserialize, Serialize};

pub const MAINTENANCE_DB: &str = "postgres";
pub const DEFAULT_CONNECTION_LIMIT: u32 = 20;
pub const CONNECTION_LIMIT_RANGE: std::ops::RangeInclusive<u32> = 1..=500;
const NAME_MAX: usize = 63;

/// The enable-list: what the panel calls it, and what `CREATE EXTENSION` calls it.
pub const EXTENSIONS: [(&str, &str); 4] = [
    ("pgvector", "vector"),
    ("pg_trgm", "pg_trgm"),
    ("pgcrypto", "pgcrypto"),
    ("uuid-ossp", "uuid-ossp"),
];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("A database called {0} already exists.")]
    Taken(String),
    #[error("No such database.")]
    NotFound,
    #[error("{0}")]
    Invalid(String),
    #[error("{0} is linked to {1}; unlink it first.")]
    Linked(String, String),
    #[error("PostgreSQL refused: {0}")]
    Host(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Database {
    pub id: String,
    pub name: String,
    pub role: String,
    pub connection_limit: u32,
    pub extensions: Vec<String>,
    pub linked_apps: Vec<String>,
    pub size_bytes: Option<i64>,
    pub connections_active: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct NewDatabase {
    pub name: String,
    pub connection_limit: Option<u32>,
    pub extensions: Vec<String>,
}

pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=NAME_MAX).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

pub fn extension_sql_name(extension: &str) -> Option<&'static str> {
    EXTENSIONS
        .iter()
        .find(|(name, _)| *name == extension)
        .map(|(_, sql)| *sql)
}

pub fn url(name: &str, role: &str, password: &str) -> String {
    format!("postgres://{role}:{password}@127.0.0.1:{PG_PORT}/{name}")
}

pub fn tunnel_command(hostname: &str) -> String {
    format!("ssh -L {PG_PORT}:127.0.0.1:{PG_PORT} root@{hostname}")
}

/// The first linked database is `DATABASE_URL`; the rest are named after themselves.
pub fn env_key(position: usize, name: &str) -> String {
    if position == 0 {
        "DATABASE_URL".to_string()
    } else {
        format!("{}_DATABASE_URL", name.to_ascii_uppercase())
    }
}

fn host_error(e: PlatformError) -> DbError {
    match e {
        PlatformError::Command { stderr, .. } => DbError::Host(
            stderr
                .lines()
                .next()
                .unwrap_or_default()
                .trim_start_matches("ERROR:")
                .trim()
                .to_string(),
        ),
        other => DbError::Host(other.to_string()),
    }
}

fn validate(new: &NewDatabase) -> Result<(), DbError> {
    if !valid_name(&new.name) {
        return Err(DbError::Invalid(
            "A database name is 1 to 63 characters of lowercase letters, digits and underscores, starting with a letter.".into(),
        ));
    }
    if let Some(limit) = new.connection_limit
        && !CONNECTION_LIMIT_RANGE.contains(&limit)
    {
        return Err(DbError::Invalid(
            "The connection limit must be between 1 and 500.".into(),
        ));
    }
    for ext in &new.extensions {
        if extension_sql_name(ext).is_none() {
            return Err(DbError::Invalid(format!(
                "{ext} is not on the extension list."
            )));
        }
    }
    Ok(())
}

pub async fn create(
    state: &State,
    platform: &dyn Platform,
    new: NewDatabase,
) -> anyhow::Result<Database> {
    validate(&new)?;
    if by_name(state, &new.name).await?.is_some() {
        return Err(DbError::Taken(new.name).into());
    }
    let name = new.name.clone();
    let role = new.name.clone();
    let limit = new.connection_limit.unwrap_or(DEFAULT_CONNECTION_LIMIT);
    let password = secret::generate();

    let mut document = sql::create_role(&role, &password, limit);
    document.push_str(&sql::create_database(&name, &role));
    document.push_str(&sql::isolate(&name, &role));
    let made = platform
        .postgres_sql(MAINTENANCE_DB, &document)
        .map_err(host_error)
        .and_then(|_| {
            for ext in &new.extensions {
                enable_on_host(platform, &name, ext)?;
            }
            Ok(())
        });
    if let Err(e) = made {
        let _ = platform.postgres_sql(MAINTENANCE_DB, &sql::drop_database(&name, &role));
        return Err(e.into());
    }

    let id = uuid::Uuid::new_v4().to_string();
    let limit = limit as i64;
    let mut tx = state.pool.begin().await?;
    sqlx::query!(
        "INSERT INTO databases (id, name, role, password, connection_limit) VALUES (?, ?, ?, ?, ?)",
        id,
        name,
        role,
        password,
        limit
    )
    .execute(&mut *tx)
    .await?;
    for ext in &new.extensions {
        sqlx::query!(
            "INSERT OR IGNORE INTO database_extensions (database_id, name) VALUES (?, ?)",
            id,
            ext
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    by_name(state, &name)
        .await?
        .ok_or_else(|| DbError::NotFound.into())
}

fn enable_on_host(platform: &dyn Platform, database: &str, extension: &str) -> Result<(), DbError> {
    let sql_name = extension_sql_name(extension)
        .ok_or_else(|| DbError::Invalid(format!("{extension} is not on the extension list.")))?;
    if extension == "pgvector" {
        let major = platform.postgres_major_installed().unwrap_or(DEFAULT_MAJOR);
        platform
            .install_packages(&[&install::extension_package(major, extension)])
            .map_err(|e| DbError::Host(format!("installing pgvector failed: {e}")))?;
    }
    platform
        .postgres_sql(database, &sql::create_extension(sql_name))
        .map(|_| ())
        .map_err(host_error)
}

pub async fn enable_extension(
    state: &State,
    platform: &dyn Platform,
    name: &str,
    extension: &str,
) -> anyhow::Result<()> {
    let db = by_name(state, name).await?.ok_or(DbError::NotFound)?;
    enable_on_host(platform, &db.name, extension)?;
    sqlx::query!(
        "INSERT OR IGNORE INTO database_extensions (database_id, name) VALUES (?, ?)",
        db.id,
        extension
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn delete(state: &State, platform: &dyn Platform, name: &str) -> anyhow::Result<()> {
    let db = by_name(state, name).await?.ok_or(DbError::NotFound)?;
    if !db.linked_apps.is_empty() {
        return Err(DbError::Linked(db.name, db.linked_apps.join(", ")).into());
    }
    platform
        .postgres_sql(MAINTENANCE_DB, &sql::drop_database(&db.name, &db.role))
        .map_err(host_error)?;
    sqlx::query!("DELETE FROM databases WHERE id = ?", db.id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

pub async fn link(state: &State, app_id: &str, name: &str) -> anyhow::Result<()> {
    let db = by_name(state, name).await?.ok_or(DbError::NotFound)?;
    sqlx::query!(
        "INSERT OR IGNORE INTO app_databases (app_id, database_id, position)
         VALUES (?, ?, (SELECT coalesce(max(position) + 1, 0) FROM app_databases WHERE app_id = ?))",
        app_id,
        db.id,
        app_id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

pub async fn unlink(state: &State, app_id: &str, name: &str) -> anyhow::Result<bool> {
    let db = by_name(state, name).await?.ok_or(DbError::NotFound)?;
    let done = sqlx::query!(
        "DELETE FROM app_databases WHERE app_id = ? AND database_id = ?",
        app_id,
        db.id
    )
    .execute(&state.pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// `(env key, url)` for every database linked to the app, in link order.
pub async fn urls_for(state: &State, app_id: &str) -> anyhow::Result<Vec<(String, String)>> {
    let rows = sqlx::query!(
        r#"SELECT d.name AS "name!", d.role AS "role!", d.password AS "password!"
           FROM app_databases l JOIN databases d ON d.id = l.database_id
           WHERE l.app_id = ? ORDER BY l.position, d.name"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .enumerate()
        .map(|(i, r)| (env_key(i, &r.name), url(&r.name, &r.role, &r.password)))
        .collect())
}

pub async fn names_for(state: &State, app_id: &str) -> anyhow::Result<Vec<String>> {
    let rows = sqlx::query!(
        r#"SELECT d.name AS "name!" FROM app_databases l JOIN databases d ON d.id = l.database_id
           WHERE l.app_id = ? ORDER BY l.position, d.name"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows.into_iter().map(|r| r.name).collect())
}

pub async fn count(state: &State) -> anyhow::Result<usize> {
    let n = sqlx::query_scalar!(r#"SELECT count(*) AS "n!: i64" FROM databases"#)
        .fetch_one(&state.pool)
        .await?;
    Ok(n as usize)
}

pub async fn by_name(state: &State, name: &str) -> anyhow::Result<Option<Database>> {
    Ok(rows(state).await?.into_iter().find(|d| d.name == name))
}

pub async fn by_id(state: &State, id: &str) -> anyhow::Result<Option<Database>> {
    Ok(rows(state).await?.into_iter().find(|d| d.id == id))
}

pub async fn linked_to(state: &State, app_id: &str) -> anyhow::Result<Vec<Database>> {
    let names = names_for(state, app_id).await?;
    let all = rows(state).await?;
    Ok(names
        .iter()
        .filter_map(|name| all.iter().find(|d| &d.name == name).cloned())
        .collect())
}

/// Sizes and connection counts come from one query against the cluster; when it cannot answer,
/// the list still does.
pub async fn list(state: &State, platform: &dyn Platform) -> anyhow::Result<Vec<Database>> {
    let mut databases = rows(state).await?;
    if databases.is_empty() {
        return Ok(databases);
    }
    if let Ok(out) = platform.postgres_sql(MAINTENANCE_DB, &sql::sizes()) {
        for (name, bytes, connections) in sql::parse_sizes(&out) {
            if let Some(db) = databases.iter_mut().find(|d| d.name == name) {
                db.size_bytes = Some(bytes);
                db.connections_active = Some(connections);
            }
        }
    }
    Ok(databases)
}

async fn rows(state: &State) -> anyhow::Result<Vec<Database>> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", name AS "name!", role AS "role!", connection_limit AS "connection_limit!",
                  created_at AS "created_at!"
           FROM databases ORDER BY name"#
    )
    .fetch_all(&state.pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let extensions = sqlx::query_scalar!(
            r#"SELECT name AS "name!" FROM database_extensions WHERE database_id = ? ORDER BY name"#,
            r.id
        )
        .fetch_all(&state.pool)
        .await?;
        let linked_apps = sqlx::query_scalar!(
            r#"SELECT a.slug AS "slug!" FROM app_databases l JOIN apps a ON a.id = l.app_id
               WHERE l.database_id = ? ORDER BY a.slug"#,
            r.id
        )
        .fetch_all(&state.pool)
        .await?;
        out.push(Database {
            id: r.id,
            name: r.name,
            role: r.role,
            connection_limit: r.connection_limit as u32,
            extensions,
            linked_apps,
            size_bytes: None,
            connections_active: None,
            created_at: time::utc(r.created_at),
        });
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::apps;
    use crate::apps::tests::{new_app, state};
    use ferrum_platform::FakePlatform;

    pub fn new(name: &str) -> NewDatabase {
        NewDatabase {
            name: name.into(),
            ..NewDatabase::default()
        }
    }

    #[test]
    fn names_are_what_postgres_and_the_shell_both_accept() {
        assert!(valid_name("ledger_prod"));
        assert!(valid_name("a"));
        assert!(!valid_name("Ledger"));
        assert!(!valid_name("ledger; drop"));
        assert!(!valid_name("1ledger"));
        assert!(!valid_name(""));
        assert!(!valid_name(&"a".repeat(64)));
    }

    #[test]
    fn the_url_and_the_tunnel_are_ready_to_paste() {
        assert_eq!(
            url("ledger_prod", "ledger_prod", "pw"),
            "postgres://ledger_prod:pw@127.0.0.1:5432/ledger_prod"
        );
        assert_eq!(
            tunnel_command("panel.example.com"),
            "ssh -L 5432:127.0.0.1:5432 root@panel.example.com"
        );
        assert_eq!(env_key(0, "ledger_prod"), "DATABASE_URL");
        assert_eq!(env_key(1, "analytics"), "ANALYTICS_DATABASE_URL");
    }

    #[test]
    fn a_generated_password_never_needs_escaping_in_a_url() {
        for _ in 0..50 {
            let pw = secret::generate();
            assert!(!pw.contains(['/', '@', ':', '?', '#', '%']), "{pw}");
        }
    }

    #[tokio::test]
    async fn creating_a_database_isolates_it_from_every_other_role() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let db = create(
            &state,
            &p,
            NewDatabase {
                name: "ledger_prod".into(),
                connection_limit: Some(30),
                extensions: vec![],
            },
        )
        .await
        .unwrap();
        let sql = p.sql().join("\n");
        let role = sql
            .find("CREATE ROLE \"ledger_prod\" LOGIN PASSWORD")
            .unwrap();
        let created = sql
            .find("CREATE DATABASE \"ledger_prod\" OWNER \"ledger_prod\"")
            .unwrap();
        let revoke = sql
            .find("REVOKE CONNECT ON DATABASE \"ledger_prod\" FROM PUBLIC")
            .unwrap();
        let grant = sql
            .find("GRANT CONNECT ON DATABASE \"ledger_prod\" TO \"ledger_prod\"")
            .unwrap();
        assert!(
            role < created && created < revoke && revoke < grant,
            "{sql}"
        );
        assert!(sql.contains("CONNECTION LIMIT 30"));
        assert!(
            !sql.contains("BEGIN"),
            "CREATE DATABASE cannot run in a transaction"
        );
        assert_eq!(db.connection_limit, 30);
        assert_eq!(db.role, "ledger_prod");
        assert!(db.created_at.ends_with('Z'));
        let stored: String =
            sqlx::query_scalar("SELECT password FROM databases WHERE name = 'ledger_prod'")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert!(sql.contains(&sql::quote_literal(&stored)));
        assert!(stored.len() >= 43);
        assert!(
            p.calls()
                .iter()
                .all(|c| !c.starts_with("postgres_sql ") || c.starts_with("postgres_sql postgres ")),
            "creation runs against the maintenance database"
        );
    }

    #[tokio::test]
    async fn a_failed_create_leaves_no_row_and_drops_what_was_made() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        p.fail_next("REVOKE CONNECT");
        let e = create(&state, &p, new("ledger_prod")).await.unwrap_err();
        assert!(e.to_string().contains("PostgreSQL refused"), "{e}");
        assert!(by_name(&state, "ledger_prod").await.unwrap().is_none());
        let cleanup = p.sql().into_iter().last().unwrap();
        assert!(cleanup.contains("DROP DATABASE IF EXISTS \"ledger_prod\" WITH (FORCE)"));
        assert!(cleanup.contains("DROP ROLE IF EXISTS \"ledger_prod\""));
    }

    #[tokio::test]
    async fn bad_names_and_unknown_extensions_never_reach_psql() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        for bad in ["Ledger", "a;b", "", "9x"] {
            assert!(create(&state, &p, new(bad)).await.is_err(), "{bad:?}");
        }
        let mut postgis = new("ok");
        postgis.extensions = vec!["postgis".into()];
        assert!(create(&state, &p, postgis).await.is_err());
        let mut limit = new("ok");
        limit.connection_limit = Some(0);
        assert!(create(&state, &p, limit).await.is_err());
        assert!(p.sql().is_empty());
    }

    #[tokio::test]
    async fn a_duplicate_name_is_a_conflict_before_anything_runs() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        create(&state, &p, new("ledger_prod")).await.unwrap();
        let before = p.sql().len();
        let e = create(&state, &p, new("ledger_prod")).await.unwrap_err();
        assert!(matches!(
            e.downcast_ref::<DbError>(),
            Some(DbError::Taken(_))
        ));
        assert_eq!(p.sql().len(), before);
    }

    #[tokio::test]
    async fn a_linked_database_cannot_be_deleted_and_unlinking_never_drops() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        create(&state, &p, new("ledger_prod")).await.unwrap();
        link(&state, &app.id, "ledger_prod").await.unwrap();
        link(&state, &app.id, "ledger_prod").await.unwrap();
        assert_eq!(
            by_name(&state, "ledger_prod")
                .await
                .unwrap()
                .unwrap()
                .linked_apps,
            vec!["ledger"]
        );
        let e = delete(&state, &p, "ledger_prod").await.unwrap_err();
        assert!(e.to_string().contains("ledger"), "{e}");
        assert!(unlink(&state, &app.id, "ledger_prod").await.unwrap());
        assert!(!unlink(&state, &app.id, "ledger_prod").await.unwrap());
        assert!(!p.sql().iter().any(|s| s.contains("DROP")));
        delete(&state, &p, "ledger_prod").await.unwrap();
        assert!(
            p.sql()
                .iter()
                .any(|s| s.contains("DROP DATABASE IF EXISTS \"ledger_prod\" WITH (FORCE)"))
        );
        assert!(by_name(&state, "ledger_prod").await.unwrap().is_none());
        assert!(delete(&state, &p, "ledger_prod").await.is_err());
    }

    #[tokio::test]
    async fn deleting_an_app_unlinks_but_keeps_the_database() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        create(&state, &p, new("ledger_prod")).await.unwrap();
        link(&state, &app.id, "ledger_prod").await.unwrap();
        apps::delete(&state, "ledger").await.unwrap();
        let db = by_name(&state, "ledger_prod").await.unwrap().unwrap();
        assert!(db.linked_apps.is_empty());
    }

    #[tokio::test]
    async fn linked_urls_are_named_in_link_order() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        create(&state, &p, new("ledger_prod")).await.unwrap();
        create(&state, &p, new("analytics")).await.unwrap();
        link(&state, &app.id, "ledger_prod").await.unwrap();
        link(&state, &app.id, "analytics").await.unwrap();
        let urls = urls_for(&state, &app.id).await.unwrap();
        assert_eq!(urls[0].0, "DATABASE_URL");
        assert!(urls[0].1.starts_with("postgres://ledger_prod:"));
        assert_eq!(urls[1].0, "ANALYTICS_DATABASE_URL");
        assert!(urls[1].1.ends_with("@127.0.0.1:5432/analytics"));
        assert_eq!(
            names_for(&state, &app.id).await.unwrap(),
            vec!["ledger_prod", "analytics"]
        );
        unlink(&state, &app.id, "ledger_prod").await.unwrap();
        assert_eq!(
            urls_for(&state, &app.id).await.unwrap()[0].0,
            "DATABASE_URL",
            "the next database moves up"
        );
    }

    #[tokio::test]
    async fn extensions_come_from_the_enable_list_only() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        p.set_postgres_major(18);
        create(&state, &p, new("ledger_prod")).await.unwrap();
        enable_extension(&state, &p, "ledger_prod", "pg_trgm")
            .await
            .unwrap();
        assert!(
            p.calls().contains(
                &"postgres_sql ledger_prod CREATE EXTENSION IF NOT EXISTS \"pg_trgm\";\n"
                    .to_string()
            ),
            "the extension is created inside the database"
        );
        assert!(
            enable_extension(&state, &p, "ledger_prod", "postgis")
                .await
                .is_err()
        );
        enable_extension(&state, &p, "ledger_prod", "pgvector")
            .await
            .unwrap();
        assert!(
            p.calls()
                .contains(&"install_packages postgresql-18-pgvector".to_string())
        );
        assert!(
            p.sql()
                .iter()
                .any(|s| s.contains("CREATE EXTENSION IF NOT EXISTS \"vector\""))
        );
        assert_eq!(
            by_name(&state, "ledger_prod")
                .await
                .unwrap()
                .unwrap()
                .extensions,
            vec!["pg_trgm", "pgvector"]
        );
    }

    #[tokio::test]
    async fn sizes_and_connections_come_from_the_cluster_when_it_answers() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        create(&state, &p, new("ledger_prod")).await.unwrap();
        assert_eq!(list(&state, &p).await.unwrap()[0].size_bytes, None);
        p.answer_sql(
            "pg_database_size",
            "postgres|7000000|1\nledger_prod|123456|3\n",
        );
        let db = &list(&state, &p).await.unwrap()[0];
        assert_eq!(db.size_bytes, Some(123_456));
        assert_eq!(db.connections_active, Some(3));
        p.fail_next("pg_database_size");
        assert_eq!(list(&state, &p).await.unwrap()[0].size_bytes, None);
    }

    #[tokio::test]
    async fn nothing_asks_the_cluster_when_there_is_nothing_to_size() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        assert!(list(&state, &p).await.unwrap().is_empty());
        assert!(p.sql().is_empty());
    }
}
