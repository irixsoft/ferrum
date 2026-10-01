pub mod install;
pub mod restore;
pub mod roles;
pub mod sql;
pub mod tune;

pub use install::{DEFAULT_MAJOR, ensure_installed, major};
pub use roles::{NewRole, Role};

use crate::apps::App;
use crate::apps::env::{ManagedVar, Origin};
use crate::manifest::DatabaseSpec;
use crate::state::State;
use crate::time;
use crate::{secret, secrets};
use ferrum_platform::ubuntu::PG_PORT;
use ferrum_platform::{Platform, PlatformError};
use serde::{Deserialize, Serialize};

pub const MAINTENANCE_DB: &str = "postgres";
pub const DEFAULT_CONNECTION_LIMIT: u32 = 20;
pub const CONNECTION_LIMIT_RANGE: std::ops::RangeInclusive<u32> = 1..=500;
const NAME_MAX: usize = 63;

/// Extensions that ship as their own apt package: `CREATE EXTENSION` name and package suffix.
pub const PACKAGED: [(&str, &str); 1] = [("vector", "pgvector")];
const ALWAYS_PRESENT: [&str; 1] = ["plpgsql"];

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
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Missing(String),
}

#[derive(Debug, Clone, Serialize)]
pub struct Database {
    pub id: String,
    pub name: String,
    pub role: String,
    pub connection_limit: u32,
    pub extensions: Vec<String>,
    pub linked_apps: Vec<String>,
    pub roles: Vec<Role>,
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
    pub env_label: Option<String>,
}

pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=NAME_MAX).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
}

pub fn valid_extension(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=NAME_MAX).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
}

/// What the server can `CREATE EXTENSION`, plus what Ferrum can install a package for.
pub fn available(platform: &dyn Platform) -> Result<Vec<String>, DbError> {
    let out = platform
        .postgres_sql(MAINTENANCE_DB, &sql::available_extensions())
        .map_err(host_error)?;
    let mut names: Vec<String> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !ALWAYS_PRESENT.contains(l))
        .map(String::from)
        .collect();
    names.extend(PACKAGED.iter().map(|(name, _)| name.to_string()));
    names.sort();
    names.dedup();
    Ok(names)
}

fn offered(platform: &dyn Platform, extensions: &[String]) -> Result<(), DbError> {
    if extensions.is_empty() {
        return Ok(());
    }
    let available = available(platform)?;
    match extensions.iter().find(|e| !available.contains(e)) {
        Some(missing) => Err(DbError::Invalid(format!(
            "This server does not offer {missing}."
        ))),
        None => Ok(()),
    }
}

pub fn url(name: &str, role: &str, password: &str) -> String {
    format!("postgres://{role}:{password}@127.0.0.1:{PG_PORT}/{name}")
}

pub fn tunnel_command(hostname: &str, user: &str) -> String {
    format!("ssh -L {PG_PORT}:127.0.0.1:{PG_PORT} {user}@{hostname}")
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
        if !valid_extension(ext) {
            return Err(DbError::Invalid(
                "An extension name is letters, digits, underscores and hyphens.".into(),
            ));
        }
    }
    if let Some(label) = &new.env_label {
        roles::valid_label(label)?;
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
    if roles::name_taken(state, &new.name).await? {
        return Err(
            DbError::Conflict(format!("A role called {} already exists.", new.name)).into(),
        );
    }
    offered(platform, &new.extensions)?;
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
        let _ = platform.postgres_sql(MAINTENANCE_DB, &sql::drop_database(&name, &[&role]));
        return Err(e.into());
    }

    let id = uuid::Uuid::new_v4().to_string();
    let limit = limit as i64;
    let sealed = secrets::encrypt(&state.key, &password);
    let mut tx = state.pool.begin().await?;
    sqlx::query!(
        "INSERT INTO databases (id, name, role, password, connection_limit) VALUES (?, ?, ?, ?, ?)",
        id,
        name,
        role,
        sealed,
        limit
    )
    .execute(&mut *tx)
    .await?;
    let label = new.env_label.as_deref().unwrap_or(roles::OWNER_LABEL);
    roles::insert_owner(&mut tx, &id, &role, &sealed, limit, label).await?;
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
    if let Some((_, package)) = PACKAGED.iter().find(|(name, _)| *name == extension) {
        let major = platform.postgres_major_installed().unwrap_or(DEFAULT_MAJOR);
        platform
            .install_packages(&[&install::extension_package(major, package)])
            .map_err(|e| DbError::Host(format!("installing {package} failed: {e}")))?;
    }
    platform
        .postgres_sql(database, &sql::create_extension(extension))
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
    if !valid_extension(extension) {
        return Err(DbError::Invalid(
            "An extension name is letters, digits, underscores and hyphens.".into(),
        )
        .into());
    }
    offered(platform, std::slice::from_ref(&extension.to_string()))?;
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
    let mut names: Vec<&str> = vec![&db.role];
    names.extend(
        db.roles
            .iter()
            .filter(|r| !r.owner)
            .map(|r| r.name.as_str()),
    );
    platform
        .postgres_sql(MAINTENANCE_DB, &sql::drop_database(&db.name, &names))
        .map_err(host_error)?;
    sqlx::query!("DELETE FROM databases WHERE id = ?", db.id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

pub async fn link(state: &State, app_id: &str, name: &str) -> anyhow::Result<()> {
    link_as(state, app_id, name, None).await
}

/// Links under `label`, or the name by link order. Every variable the link adds must be free
/// in the app, and the database's own roles must not share one.
pub async fn link_as(
    state: &State,
    app_id: &str,
    name: &str,
    label: Option<&str>,
) -> anyhow::Result<()> {
    let db = by_name(state, name).await?.ok_or(DbError::NotFound)?;
    let linked = names_for(state, app_id).await?;
    if linked.iter().any(|n| n == name) {
        return Ok(());
    }
    if let Some(label) = label {
        roles::valid_label(label)?;
    }
    let owner_key = label
        .map(str::to_string)
        .unwrap_or_else(|| env_key(linked.len(), name));
    let mut adding = vec![(
        owner_key,
        Origin::Owner {
            database: name.to_string(),
        },
    )];
    for role in roles::list_for(state, &db.id).await? {
        if !role.owner {
            adding.push((
                role.env_label,
                Origin::Role {
                    database: name.to_string(),
                    role: role.name,
                },
            ));
        }
    }
    let carried = managed_keys(state, app_id).await?;
    for (i, (key, _)) in adding.iter().enumerate() {
        not_a_process_port(state, app_id, key).await?;
        let taken = carried
            .iter()
            .chain(adding[..i].iter())
            .find(|(k, _)| k == key);
        if let Some((_, other)) = taken {
            return Err(clash_sentence(key, other).into());
        }
    }
    sqlx::query!(
        "INSERT INTO app_databases (app_id, database_id, env_label, position)
         VALUES (?, ?, ?, (SELECT coalesce(max(position) + 1, 0) FROM app_databases WHERE app_id = ?))",
        app_id,
        db.id,
        label,
        app_id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Links, then applies what the repo's `[database]` section says about labels and roles.
pub async fn link_with_labels(
    state: &State,
    platform: &dyn Platform,
    app: &App,
    name: &str,
    spec: Option<&DatabaseSpec>,
) -> anyhow::Result<()> {
    link_as(state, &app.id, name, spec.and_then(|s| s.url.as_deref())).await?;
    let Some(spec) = spec else {
        return Ok(());
    };
    let db = by_name(state, name).await?.ok_or(DbError::NotFound)?;
    roles::ensure_from_manifest(state, platform, &db, spec, app).await?;
    if let Some(url) = &spec.url {
        set_link_label(state, &app.id, name, Some(url)).await?;
    }
    Ok(())
}

/// The owner's variable name in this app's env; `None` goes back to the name by link order.
pub async fn set_link_label(
    state: &State,
    app_id: &str,
    name: &str,
    label: Option<&str>,
) -> anyhow::Result<()> {
    if let Some(label) = label {
        roles::valid_label(label)?;
    }
    let db = by_name(state, name).await?.ok_or(DbError::NotFound)?;
    let linked = names_for(state, app_id).await?;
    let Some(position) = linked.iter().position(|n| n == name) else {
        return Err(DbError::Missing(format!("{name} is not linked to that application.")).into());
    };
    let key = label
        .map(str::to_string)
        .unwrap_or_else(|| env_key(position, name));
    let own = Origin::Owner {
        database: name.to_string(),
    };
    if let Some(other) = label_clash(state, app_id, &key, std::slice::from_ref(&own)).await? {
        return Err(clash_sentence(&key, &other).into());
    }
    let done = sqlx::query!(
        "UPDATE app_databases SET env_label = ? WHERE app_id = ? AND database_id = ?",
        label,
        app_id,
        db.id
    )
    .execute(&state.pool)
    .await?;
    if done.rows_affected() == 0 {
        return Err(DbError::Missing(format!("{name} is not linked to that application.")).into());
    }
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

/// Every database linked to the app in link order: its owner under the link's label, then each
/// of its other roles under the role's own.
pub async fn urls_for(state: &State, app_id: &str) -> anyhow::Result<Vec<ManagedVar>> {
    let rows = sqlx::query!(
        r#"SELECT d.id AS "id!", d.name AS "name!", d.role AS "role!", d.password AS "password!",
                  l.env_label
           FROM app_databases l JOIN databases d ON d.id = l.database_id
           WHERE l.app_id = ? ORDER BY l.position, d.name"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    let mut out = Vec::new();
    for (i, r) in rows.into_iter().enumerate() {
        let password = secrets::decrypt(&state.key, &r.password)?;
        out.push(ManagedVar {
            key: r.env_label.unwrap_or_else(|| env_key(i, &r.name)),
            value: url(&r.name, &r.role, &password),
            origin: Origin::Owner {
                database: r.name.clone(),
            },
        });
        for (label, role, sealed) in roles::sealed_for(state, &r.id).await? {
            let password = secrets::decrypt(&state.key, &sealed)?;
            out.push(ManagedVar {
                key: label,
                value: url(&r.name, &role, &password),
                origin: Origin::Role {
                    database: r.name.clone(),
                    role,
                },
            });
        }
    }
    Ok(out)
}

/// Every variable Ferrum renders into the app's env file, with where each comes from.
pub async fn managed_keys(state: &State, app_id: &str) -> anyhow::Result<Vec<(String, Origin)>> {
    let mut keys: Vec<(String, Origin)> = urls_for(state, app_id)
        .await?
        .into_iter()
        .map(|v| (v.key, v.origin))
        .collect();
    if let Some((key, _)) = crate::redis::url_for(state, app_id).await? {
        keys.push((key, Origin::Redis));
    }
    Ok(keys)
}

/// `<NAME>_PORT` of a listening process is Ferrum's in that app's env; any other name is free.
async fn not_a_process_port(state: &State, app_id: &str, label: &str) -> anyhow::Result<()> {
    let names = sqlx::query_scalar!(
        r#"SELECT name AS "name!" FROM app_processes WHERE app_id = ? AND has_port = 1"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    match names
        .into_iter()
        .find(|n| crate::apps::env::port_var(n) == label)
    {
        Some(name) => Err(DbError::Invalid(format!(
            "{label} is the port of the process {name}; pick another label."
        ))
        .into()),
        None => Ok(()),
    }
}

/// Who already carries `label` in the app, leaving out the origins that are being renamed; a
/// label that is a process's port name is refused outright.
pub async fn label_clash(
    state: &State,
    app_id: &str,
    label: &str,
    except: &[Origin],
) -> anyhow::Result<Option<Origin>> {
    not_a_process_port(state, app_id, label).await?;
    Ok(managed_keys(state, app_id)
        .await?
        .into_iter()
        .find(|(key, origin)| key == label && !except.contains(origin))
        .map(|(_, origin)| origin))
}

pub fn clash_sentence(label: &str, origin: &Origin) -> DbError {
    DbError::Conflict(match origin {
        Origin::Owner { database } => format!("{label} is already the label of {database}."),
        Origin::Role { database, role } => {
            format!("{label} is already the label of {role}, a role of {database}.")
        }
        Origin::Redis => format!("{label} is already the label of Redis."),
    })
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
        let roles = roles::list_for(state, &r.id).await?;
        out.push(Database {
            id: r.id,
            name: r.name,
            role: r.role,
            connection_limit: r.connection_limit as u32,
            extensions,
            linked_apps,
            roles,
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
            tunnel_command("panel.example.com", "ubuntu"),
            "ssh -L 5432:127.0.0.1:5432 ubuntu@panel.example.com"
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
                env_label: None,
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
        assert!(secrets::is_encrypted(&stored));
        let password = secrets::decrypt(&state.key, &stored).unwrap();
        assert!(sql.contains(&sql::quote_literal(&password)));
        assert!(password.len() >= 43);
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
    async fn bad_names_and_unoffered_extensions_never_create_anything() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        for bad in ["Ledger", "a;b", "", "9x"] {
            assert!(create(&state, &p, new(bad)).await.is_err(), "{bad:?}");
        }
        let mut odd = new("ok");
        odd.extensions = vec!["post gis".into()];
        assert!(create(&state, &p, odd).await.is_err());
        let mut limit = new("ok");
        limit.connection_limit = Some(0);
        assert!(create(&state, &p, limit).await.is_err());
        assert!(p.sql().is_empty());

        p.answer_sql("pg_available_extensions", "citext\nplpgsql\n");
        let mut postgis = new("ok");
        postgis.extensions = vec!["postgis".into()];
        let e = create(&state, &p, postgis).await.unwrap_err();
        assert_eq!(e.to_string(), "This server does not offer postgis.");
        assert!(
            p.sql()
                .iter()
                .all(|s| s.starts_with("SELECT name FROM pg_available_extensions")),
            "{:?}",
            p.sql()
        );
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
        assert_eq!(urls[0].key, "DATABASE_URL");
        assert!(urls[0].value.starts_with("postgres://ledger_prod:"));
        assert_eq!(urls[1].key, "ANALYTICS_DATABASE_URL");
        assert!(urls[1].value.ends_with("@127.0.0.1:5432/analytics"));
        assert_eq!(
            names_for(&state, &app.id).await.unwrap(),
            vec!["ledger_prod", "analytics"]
        );
        unlink(&state, &app.id, "ledger_prod").await.unwrap();
        assert_eq!(
            urls_for(&state, &app.id).await.unwrap()[0].key,
            "DATABASE_URL",
            "the next database moves up"
        );
    }

    #[tokio::test]
    async fn a_link_label_names_the_owner_and_each_role_follows_under_its_own() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let mut labelled = new("ledger_prod");
        labelled.env_label = Some("DATABASE_ADMIN_URL".into());
        let db = create(&state, &p, labelled).await.unwrap();
        assert_eq!(db.roles.len(), 1);
        assert!(db.roles[0].owner);
        assert_eq!(db.roles[0].env_label, "DATABASE_ADMIN_URL");
        roles::create(
            &state,
            &p,
            &db,
            NewRole {
                name: "app".into(),
                env_label: Some("DATABASE_URL".into()),
                connection_limit: None,
            },
        )
        .await
        .unwrap();
        let refused = link(&state, &app.id, "ledger_prod").await.unwrap_err();
        assert_eq!(
            refused.to_string(),
            "DATABASE_URL is already the label of ledger_prod.",
            "an unlabelled link would name the owner DATABASE_URL as well"
        );
        link_as(&state, &app.id, "ledger_prod", Some("DATABASE_ADMIN_URL"))
            .await
            .unwrap();
        let keys =
            |vars: Vec<ManagedVar>| -> Vec<String> { vars.into_iter().map(|v| v.key).collect() };
        let vars = urls_for(&state, &app.id).await.unwrap();
        assert_eq!(keys(vars.clone()), ["DATABASE_ADMIN_URL", "DATABASE_URL"]);
        assert!(vars[1].value.starts_with("postgres://ledger_prod_app:"));
        assert_eq!(
            vars[1].origin,
            Origin::Role {
                database: "ledger_prod".into(),
                role: "ledger_prod_app".into()
            }
        );
        assert!(
            set_link_label(&state, &app.id, "ledger_prod", Some("1BAD"))
                .await
                .is_err()
        );
        let process = app.port_processes().next().unwrap().name.clone();
        let port = apps::env::port_var(&process);
        assert_eq!(
            set_link_label(&state, &app.id, "ledger_prod", Some(&port))
                .await
                .unwrap_err()
                .to_string(),
            format!("{port} is the port of the process {process}; pick another label.")
        );
        set_link_label(&state, &app.id, "ledger_prod", Some("LEGACY_PORT"))
            .await
            .unwrap();
        let clash = set_link_label(&state, &app.id, "ledger_prod", None)
            .await
            .unwrap_err();
        assert_eq!(
            clash.to_string(),
            "DATABASE_URL is already the label of ledger_prod_app, a role of ledger_prod."
        );
        roles::set_labels(&state, &db, &[("app".into(), "DATABASE_APP_URL".into())])
            .await
            .unwrap();
        set_link_label(&state, &app.id, "ledger_prod", None)
            .await
            .unwrap();
        assert_eq!(
            keys(urls_for(&state, &app.id).await.unwrap()),
            ["DATABASE_URL", "DATABASE_APP_URL"]
        );
    }

    #[tokio::test]
    async fn extensions_come_from_what_the_server_offers_plus_the_packaged_ones() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        p.set_postgres_major(18);
        p.answer_sql(
            "pg_available_extensions",
            "citext\npg_trgm\nplpgsql\nuuid-ossp\n",
        );
        assert_eq!(
            available(&p).unwrap(),
            vec!["citext", "pg_trgm", "uuid-ossp", "vector"],
            "plpgsql is always there and never offered; vector comes with its package"
        );
        create(&state, &p, new("ledger_prod")).await.unwrap();
        enable_extension(&state, &p, "ledger_prod", "citext")
            .await
            .unwrap();
        assert!(
            p.calls().contains(
                &"postgres_sql ledger_prod CREATE EXTENSION IF NOT EXISTS \"citext\";\n"
                    .to_string()
            ),
            "the extension is created inside the database"
        );
        let e = enable_extension(&state, &p, "ledger_prod", "postgis")
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), "This server does not offer postgis.");
        enable_extension(&state, &p, "ledger_prod", "vector")
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
            vec!["citext", "vector"]
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
