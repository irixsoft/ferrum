use super::{
    CONNECTION_LIMIT_RANGE, DEFAULT_CONNECTION_LIMIT, Database, DbError, MAINTENANCE_DB, NAME_MAX,
    host_error, sql,
};
use crate::apps::{App, env, processes};
use crate::events::{self, Kind};
use crate::manifest::DatabaseSpec;
use crate::state::State;
use crate::{secret, secrets, time};
use ferrum_platform::Platform;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

pub const OWNER_LABEL: &str = "DATABASE_URL";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Role {
    pub id: String,
    pub database_id: String,
    pub name: String,
    pub env_label: String,
    pub connection_limit: u32,
    pub owner: bool,
    pub bypass_rls: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct NewRole {
    pub name: String,
    pub env_label: Option<String>,
    pub connection_limit: Option<u32>,
}

pub fn role_name(database: &str, name: &str) -> String {
    format!("{database}_{name}")
}

pub fn default_label(name: &str) -> String {
    format!("DATABASE_URL_{}", name.to_ascii_uppercase())
}

/// A label becomes a variable in the app's env file, next to the ones Ferrum sets per unit.
pub fn valid_label(label: &str) -> Result<(), DbError> {
    if env::valid_key(label).is_err() {
        return Err(DbError::Invalid(format!(
            "{label:?} is not a valid variable name; use letters, digits and underscores."
        )));
    }
    if label == "PORT" || label == "HOST" || label.ends_with("_PORT") {
        return Err(DbError::Invalid(format!(
            "{label} is set by Ferrum for every process; pick another label."
        )));
    }
    Ok(())
}

pub async fn list_for(state: &State, database_id: &str) -> anyhow::Result<Vec<Role>> {
    let rows = sqlx::query!(
        r#"SELECT id AS "id!", database_id AS "database_id!", name AS "name!",
                  env_label AS "env_label!", connection_limit AS "connection_limit!",
                  owner AS "owner!: bool", bypass_rls AS "bypass_rls!: bool",
                  created_at AS "created_at!"
           FROM database_roles WHERE database_id = ? ORDER BY owner DESC, name"#,
        database_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Role {
            id: r.id,
            database_id: r.database_id,
            name: r.name,
            env_label: r.env_label,
            connection_limit: r.connection_limit as u32,
            owner: r.owner,
            bypass_rls: r.bypass_rls,
            created_at: time::utc(r.created_at),
        })
        .collect())
}

/// Only the owner can bypass row-level security, and only while the file asks for it; a table
/// with `FORCE ROW LEVEL SECURITY` otherwise hides its rows from the owner too.
async fn set_owner_bypass_rls(
    state: &State,
    platform: &dyn Platform,
    db: &Database,
    on: bool,
) -> anyhow::Result<()> {
    let owner = found(state, db, &db.role).await?;
    if owner.bypass_rls == on {
        return Ok(());
    }
    platform
        .postgres_sql(MAINTENANCE_DB, &sql::set_bypass_rls(&db.role, on))
        .map_err(host_error)?;
    sqlx::query!(
        "UPDATE database_roles SET bypass_rls = ? WHERE id = ?",
        on,
        owner.id
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// `role` is the full role name or the part after `<database>_`.
pub async fn find(state: &State, db: &Database, role: &str) -> anyhow::Result<Option<Role>> {
    let full = role_name(&db.name, role);
    Ok(list_for(state, &db.id)
        .await?
        .into_iter()
        .find(|r| r.name == role || (!r.owner && r.name == full)))
}

async fn found(state: &State, db: &Database, role: &str) -> anyhow::Result<Role> {
    find(state, db, role)
        .await?
        .ok_or_else(|| DbError::Missing(format!("{} has no role called {role}.", db.name)).into())
}

pub async fn name_taken(state: &State, name: &str) -> anyhow::Result<bool> {
    let n = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!: i64" FROM database_roles WHERE name = ?"#,
        name
    )
    .fetch_one(&state.pool)
    .await?;
    Ok(n > 0)
}

pub(super) async fn insert_owner(
    tx: &mut Transaction<'_, Sqlite>,
    database_id: &str,
    name: &str,
    sealed: &str,
    limit: i64,
    label: &str,
) -> anyhow::Result<()> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query!(
        "INSERT INTO database_roles (id, database_id, name, password, connection_limit, env_label, owner)
         VALUES (?, ?, ?, ?, ?, ?, 1)",
        id,
        database_id,
        name,
        sealed,
        limit,
        label
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn validate(db: &Database, new: &NewRole) -> Result<(String, String, u32), DbError> {
    let role = role_name(&db.name, &new.name);
    if !processes::valid_name(&new.name) || role.len() > NAME_MAX {
        return Err(DbError::Invalid(format!(
            "A role name is lowercase letters, digits and underscores, starting with a letter; with the database's name in front it fits in {NAME_MAX} characters."
        )));
    }
    let label = new
        .env_label
        .clone()
        .unwrap_or_else(|| default_label(&new.name));
    valid_label(&label)?;
    let limit = new.connection_limit.unwrap_or(DEFAULT_CONNECTION_LIMIT);
    if !CONNECTION_LIMIT_RANGE.contains(&limit) {
        return Err(DbError::Invalid(
            "The connection limit must be between 1 and 500.".into(),
        ));
    }
    Ok((role, label, limit))
}

pub async fn create(
    state: &State,
    platform: &dyn Platform,
    db: &Database,
    new: NewRole,
) -> anyhow::Result<Role> {
    let (role, label, limit) = validate(db, &new)?;
    if name_taken(state, &role).await? {
        return Err(DbError::Conflict(format!("A role called {role} already exists.")).into());
    }
    if label_holder(state, &db.id, &label).await?.is_some() {
        return Err(label_taken(db, &label).into());
    }
    free_in_linked_apps(state, db, &label, &[]).await?;
    let password = secret::generate();
    let mut document = sql::create_role(&role, &password, limit);
    document.push_str(&sql::grant_connect(&db.name, &role));
    platform
        .postgres_sql(MAINTENANCE_DB, &document)
        .map_err(host_error)?;

    let id = uuid::Uuid::new_v4().to_string();
    let sealed = secrets::encrypt(&state.key, &password);
    let stored_limit = limit as i64;
    sqlx::query!(
        "INSERT INTO database_roles (id, database_id, name, password, connection_limit, env_label, owner)
         VALUES (?, ?, ?, ?, ?, ?, 0)",
        id,
        db.id,
        role,
        sealed,
        stored_limit,
        label
    )
    .execute(&state.pool)
    .await?;
    found(state, db, &role).await
}

pub async fn rotate(
    state: &State,
    platform: &dyn Platform,
    db: &Database,
    role: &str,
) -> anyhow::Result<Role> {
    let target = found(state, db, role).await?;
    let password = secret::generate();
    platform
        .postgres_sql(
            MAINTENANCE_DB,
            &sql::alter_password(&target.name, &password),
        )
        .map_err(host_error)?;
    let sealed = secrets::encrypt(&state.key, &password);
    let mut tx = state.pool.begin().await?;
    sqlx::query!(
        "UPDATE database_roles SET password = ? WHERE id = ?",
        sealed,
        target.id
    )
    .execute(&mut *tx)
    .await?;
    if target.owner {
        sqlx::query!(
            "UPDATE databases SET password = ? WHERE id = ?",
            sealed,
            db.id
        )
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(target)
}

pub async fn remove(
    state: &State,
    platform: &dyn Platform,
    db: &Database,
    role: &str,
) -> anyhow::Result<()> {
    let target = found(state, db, role).await?;
    if target.owner {
        return Err(DbError::Invalid(format!(
            "{} owns {}; it goes only when the database does.",
            target.name, db.name
        ))
        .into());
    }
    platform
        .postgres_sql(&db.name, &sql::drop_role(&target.name, &db.role))
        .map_err(host_error)?;
    sqlx::query!("DELETE FROM database_roles WHERE id = ?", target.id)
        .execute(&state.pool)
        .await?;
    Ok(())
}

pub async fn url(state: &State, db: &Database, role: &str) -> anyhow::Result<String> {
    let target = found(state, db, role).await?;
    let sealed = sqlx::query_scalar!(
        r#"SELECT password AS "password!" FROM database_roles WHERE id = ?"#,
        target.id
    )
    .fetch_one(&state.pool)
    .await?;
    let password = secrets::decrypt(&state.key, &sealed)?;
    Ok(super::url(&db.name, &target.name, &password))
}

/// `(label, role, sealed password)` for every role but the owner, by name.
pub(crate) async fn sealed_for(
    state: &State,
    database_id: &str,
) -> anyhow::Result<Vec<(String, String, String)>> {
    let rows = sqlx::query!(
        r#"SELECT env_label AS "env_label!", name AS "name!", password AS "password!"
           FROM database_roles WHERE database_id = ? AND owner = 0 ORDER BY name"#,
        database_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.env_label, r.name, r.password))
        .collect())
}

/// A recreated database has lost its grants; the roles still exist in the cluster.
pub async fn regrant(state: &State, platform: &dyn Platform, db: &Database) -> anyhow::Result<()> {
    let document: String = list_for(state, &db.id)
        .await?
        .iter()
        .filter(|r| !r.owner)
        .map(|r| sql::grant_connect(&db.name, &r.name))
        .collect();
    if document.is_empty() {
        return Ok(());
    }
    platform
        .postgres_sql(MAINTENANCE_DB, &document)
        .map_err(host_error)?;
    Ok(())
}

async fn label_holder(
    state: &State,
    database_id: &str,
    label: &str,
) -> anyhow::Result<Option<String>> {
    Ok(sqlx::query_scalar!(
        r#"SELECT name AS "name!" FROM database_roles WHERE database_id = ? AND env_label = ?"#,
        database_id,
        label
    )
    .fetch_optional(&state.pool)
    .await?)
}

async fn linked_app_ids(state: &State, database_id: &str) -> anyhow::Result<Vec<String>> {
    Ok(sqlx::query_scalar!(
        r#"SELECT app_id AS "app_id!" FROM app_databases WHERE database_id = ?"#,
        database_id
    )
    .fetch_all(&state.pool)
    .await?)
}

/// A label is one variable in every app the database is linked to, so it must be free in each.
async fn free_in_linked_apps(
    state: &State,
    db: &Database,
    label: &str,
    except: &[env::Origin],
) -> anyhow::Result<()> {
    for app_id in linked_app_ids(state, &db.id).await? {
        if let Some(other) = super::label_clash(state, &app_id, label, except).await? {
            return Err(super::clash_sentence(label, &other).into());
        }
    }
    Ok(())
}

fn label_taken(db: &Database, label: &str) -> DbError {
    DbError::Conflict(format!(
        "{label} is already the label of another role of {}.",
        db.name
    ))
}

/// Sets several labels of one database at once, so two roles can trade theirs.
pub async fn set_labels(
    state: &State,
    db: &Database,
    changes: &[(String, String)],
) -> anyhow::Result<()> {
    let mut seen = Vec::new();
    for (_, label) in changes {
        valid_label(label)?;
        if seen.contains(&label) {
            return Err(DbError::Invalid(format!("{label} is named twice.")).into());
        }
        seen.push(label);
    }
    let mut targets = Vec::with_capacity(changes.len());
    for (role, label) in changes {
        targets.push((found(state, db, role).await?, label));
    }
    let moving: Vec<env::Origin> = targets
        .iter()
        .map(|(role, _)| {
            if role.owner {
                env::Origin::Owner {
                    database: db.name.clone(),
                }
            } else {
                env::Origin::Role {
                    database: db.name.clone(),
                    role: role.name.clone(),
                }
            }
        })
        .collect();
    for (_, label) in &targets {
        free_in_linked_apps(state, db, label, &moving).await?;
    }
    let mut tx = state.pool.begin().await?;
    for (role, label) in &targets {
        if &&role.env_label != label {
            let parked = format!("~{}", role.id);
            sqlx::query!(
                "UPDATE database_roles SET env_label = ? WHERE id = ?",
                parked,
                role.id
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    for (role, label) in &targets {
        let label = label.as_str();
        let done = sqlx::query!(
            "UPDATE database_roles SET env_label = ? WHERE id = ?",
            label,
            role.id
        )
        .execute(&mut *tx)
        .await;
        if let Err(sqlx::Error::Database(e)) = &done
            && e.is_unique_violation()
        {
            return Err(label_taken(db, label).into());
        }
        done?;
    }
    tx.commit().await?;
    Ok(())
}

/// Creates the roles the file names that do not exist yet and gives each named one its label; a
/// stored role the file no longer names keeps working, and the app gets a notice about it.
pub async fn ensure_from_manifest(
    state: &State,
    platform: &dyn Platform,
    db: &Database,
    spec: &DatabaseSpec,
    app: &App,
) -> anyhow::Result<()> {
    let mut labels: Vec<String> = spec.url.iter().cloned().collect();
    for (name, role) in &spec.roles {
        validate(
            db,
            &NewRole {
                name: name.clone(),
                env_label: role.url.clone(),
                connection_limit: None,
            },
        )?;
        let label = role.url.clone().unwrap_or_else(|| default_label(name));
        if labels.contains(&label) {
            return Err(DbError::Invalid(format!("ferrum.toml names {label} twice.")).into());
        }
        labels.push(label);
    }
    if let Some(url) = &spec.url {
        valid_label(url)?;
    }

    let mut changes = Vec::new();
    if let Some(url) = &spec.url {
        changes.push((db.role.clone(), url.clone()));
    }
    for (name, role) in &spec.roles {
        let label = role.url.clone().unwrap_or_else(|| default_label(name));
        match find(state, db, name).await? {
            Some(stored) => {
                if role.url.is_some() {
                    changes.push((stored.name, label));
                }
            }
            None => {
                let holder = label_holder(state, &db.id, &label).await?;
                if let Some(holder) = &holder {
                    let moves = (spec.url.is_some() && *holder == db.role)
                        || spec
                            .roles
                            .keys()
                            .any(|named| role_name(&db.name, named) == *holder);
                    if !moves {
                        return Err(label_taken(db, &label).into());
                    }
                }
                let free = holder.is_none();
                let first = if free {
                    label.clone()
                } else {
                    format!("PENDING_{}", uuid::Uuid::new_v4().simple())
                };
                let made = create(
                    state,
                    platform,
                    db,
                    NewRole {
                        name: name.clone(),
                        env_label: Some(first),
                        connection_limit: None,
                    },
                )
                .await?;
                if !free {
                    changes.push((made.name, label));
                }
            }
        }
    }
    set_labels(state, db, &changes).await?;
    set_owner_bypass_rls(state, platform, db, spec.bypass_rls).await?;

    for stored in list_for(state, &db.id).await? {
        let named = spec
            .roles
            .keys()
            .any(|name| role_name(&db.name, name) == stored.name);
        if stored.owner || named {
            continue;
        }
        events::emit(
            state,
            Kind::RoleKept,
            Some(&app.id),
            &stored.name,
            &format!(
                "{}: ferrum.toml no longer names the role {} of {}. It keeps working and keeps {}; remove it on the Databases page when nothing uses it.",
                app.slug, stored.name, db.name, stored.env_label
            ),
            Some("/databases"),
        )
        .await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::manifest::RoleSpec;
    use crate::postgres::{self, tests::new};
    use ferrum_platform::FakePlatform;

    fn new_role(name: &str) -> NewRole {
        NewRole {
            name: name.into(),
            ..NewRole::default()
        }
    }

    async fn ledger(state: &State, p: &FakePlatform) -> Database {
        postgres::create(state, p, new("ledger_prod"))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_role_is_created_with_connect_only_and_its_password_sealed() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let db = ledger(&state, &p).await;
        let before = p.sql().len();
        let role = create(
            &state,
            &p,
            &db,
            NewRole {
                name: "app".into(),
                env_label: None,
                connection_limit: Some(40),
            },
        )
        .await
        .unwrap();
        assert_eq!(role.name, "ledger_prod_app");
        assert_eq!(role.env_label, "DATABASE_URL_APP");
        assert_eq!(role.connection_limit, 40);
        assert!(!role.owner);
        let sql = &p.sql()[before..];
        assert_eq!(sql.len(), 1, "{sql:?}");
        let stored: String = sqlx::query_scalar(
            "SELECT password FROM database_roles WHERE name = 'ledger_prod_app'",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
        assert!(secrets::is_encrypted(&stored));
        let password = secrets::decrypt(&state.key, &stored).unwrap();
        assert_eq!(
            sql[0],
            format!(
                "CREATE ROLE \"ledger_prod_app\" LOGIN PASSWORD {} CONNECTION LIMIT 40;\nGRANT CONNECT ON DATABASE \"ledger_prod\" TO \"ledger_prod_app\";\n",
                sql::quote_literal(&password)
            )
        );
        assert!(
            p.calls_matching("postgres_sql postgres CREATE ROLE")
                .iter()
                .any(|c| c.contains("ledger_prod_app"))
        );
        assert_eq!(
            url(&state, &db, "app").await.unwrap(),
            format!("postgres://ledger_prod_app:{password}@127.0.0.1:5432/ledger_prod")
        );
        let names: Vec<String> = list_for(&state, &db.id)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(names, ["ledger_prod", "ledger_prod_app"], "the owner first");
        assert!(!serde_json::to_string(&role).unwrap().contains("password"));
    }

    #[tokio::test]
    async fn bad_names_labels_and_duplicates_never_reach_psql() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let db = ledger(&state, &p).await;
        create(&state, &p, &db, new_role("app")).await.unwrap();
        let before = p.sql().len();
        for bad in ["App", "a-b", "", "redis", &"a".repeat(60)] {
            assert!(
                create(&state, &p, &db, new_role(bad)).await.is_err(),
                "{bad:?}"
            );
        }
        for label in ["1X", "HOST", "WEB_PORT", "A B"] {
            let e = create(
                &state,
                &p,
                &db,
                NewRole {
                    name: "reader".into(),
                    env_label: Some(label.into()),
                    connection_limit: None,
                },
            )
            .await;
            assert!(e.is_err(), "{label:?}");
        }
        let twice = create(&state, &p, &db, new_role("app")).await.unwrap_err();
        assert!(matches!(
            twice.downcast_ref::<DbError>(),
            Some(DbError::Conflict(_))
        ));
        let owner_label = create(
            &state,
            &p,
            &db,
            NewRole {
                name: "reader".into(),
                env_label: Some("DATABASE_URL".into()),
                connection_limit: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            owner_label.to_string(),
            "DATABASE_URL is already the label of another role of ledger_prod."
        );
        assert_eq!(p.sql().len(), before);
    }

    #[tokio::test]
    async fn a_role_name_cannot_take_another_databases_owner() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        postgres::create(&state, &p, new("ledger_prod"))
            .await
            .unwrap();
        let ledger = postgres::create(&state, &p, new("ledger")).await.unwrap();
        let e = create(&state, &p, &ledger, new_role("prod"))
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), "A role called ledger_prod already exists.");
        create(&state, &p, &ledger, new_role("app")).await.unwrap();
        let e = postgres::create(&state, &p, new("ledger_app"))
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), "A role called ledger_app already exists.");
    }

    #[tokio::test]
    async fn rotating_changes_the_password_in_the_cluster_and_the_row() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let db = ledger(&state, &p).await;
        create(&state, &p, &db, new_role("app")).await.unwrap();
        let before = url(&state, &db, "ledger_prod_app").await.unwrap();
        rotate(&state, &p, &db, "app").await.unwrap();
        let after = url(&state, &db, "app").await.unwrap();
        assert_ne!(before, after);
        let password = after.split(':').nth(2).unwrap().split('@').next().unwrap();
        assert_eq!(
            p.sql().last().unwrap(),
            &format!("ALTER ROLE \"ledger_prod_app\" PASSWORD '{password}';\n")
        );

        let owner_before = url(&state, &db, "ledger_prod").await.unwrap();
        rotate(&state, &p, &db, "ledger_prod").await.unwrap();
        let owner_after = url(&state, &db, "ledger_prod").await.unwrap();
        assert_ne!(owner_before, owner_after);
        let sealed: String =
            sqlx::query_scalar("SELECT password FROM databases WHERE name = 'ledger_prod'")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        let password = secrets::decrypt(&state.key, &sealed).unwrap();
        assert!(
            owner_after.contains(&password),
            "the owner's copy moves too"
        );
    }

    #[tokio::test]
    async fn removing_reassigns_inside_the_database_and_the_owner_stays() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let db = ledger(&state, &p).await;
        create(&state, &p, &db, new_role("app")).await.unwrap();
        let e = remove(&state, &p, &db, "ledger_prod").await.unwrap_err();
        assert_eq!(
            e.to_string(),
            "ledger_prod owns ledger_prod; it goes only when the database does."
        );
        remove(&state, &p, &db, "app").await.unwrap();
        assert_eq!(
            p.calls().last().unwrap(),
            "postgres_sql ledger_prod REASSIGN OWNED BY \"ledger_prod_app\" TO \"ledger_prod\";\nDROP OWNED BY \"ledger_prod_app\";\nDROP ROLE IF EXISTS \"ledger_prod_app\";\n"
        );
        assert_eq!(list_for(&state, &db.id).await.unwrap().len(), 1);
        assert!(remove(&state, &p, &db, "app").await.is_err());
    }

    #[tokio::test]
    async fn deleting_the_database_drops_every_role() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let db = ledger(&state, &p).await;
        create(&state, &p, &db, new_role("app")).await.unwrap();
        postgres::delete(&state, &p, "ledger_prod").await.unwrap();
        assert_eq!(
            p.sql().last().unwrap(),
            "DROP DATABASE IF EXISTS \"ledger_prod\" WITH (FORCE);\nDROP ROLE IF EXISTS \"ledger_prod\";\nDROP ROLE IF EXISTS \"ledger_prod_app\";\n"
        );
        let left: i64 = sqlx::query_scalar("SELECT count(*) FROM database_roles")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }

    #[tokio::test]
    async fn regrant_gives_connect_back_to_every_role_but_the_owner() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let db = ledger(&state, &p).await;
        let before = p.sql().len();
        regrant(&state, &p, &db).await.unwrap();
        assert_eq!(p.sql().len(), before, "nothing to grant, nothing asked");
        create(&state, &p, &db, new_role("app")).await.unwrap();
        create(&state, &p, &db, new_role("reader")).await.unwrap();
        regrant(&state, &p, &db).await.unwrap();
        assert_eq!(
            p.calls().last().unwrap(),
            "postgres_sql postgres GRANT CONNECT ON DATABASE \"ledger_prod\" TO \"ledger_prod_app\";\nGRANT CONNECT ON DATABASE \"ledger_prod\" TO \"ledger_prod_reader\";\n"
        );
    }

    #[tokio::test]
    async fn two_roles_can_trade_labels_and_a_clash_is_a_sentence() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let db = ledger(&state, &p).await;
        create(&state, &p, &db, new_role("app")).await.unwrap();
        create(&state, &p, &db, new_role("reader")).await.unwrap();
        set_labels(
            &state,
            &db,
            &[
                ("app".into(), "DATABASE_URL_READER".into()),
                ("reader".into(), "DATABASE_URL_APP".into()),
            ],
        )
        .await
        .unwrap();
        let app = find(&state, &db, "app").await.unwrap().unwrap();
        assert_eq!(app.env_label, "DATABASE_URL_READER");
        let clash = set_labels(&state, &db, &[("app".into(), "DATABASE_URL".into())])
            .await
            .unwrap_err();
        assert_eq!(
            clash.to_string(),
            "DATABASE_URL is already the label of another role of ledger_prod."
        );
        let app = find(&state, &db, "app").await.unwrap().unwrap();
        assert_eq!(app.env_label, "DATABASE_URL_READER", "nothing half-applied");
    }

    #[tokio::test]
    async fn a_label_an_app_already_carries_from_elsewhere_is_refused() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = crate::apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let db = ledger(&state, &p).await;
        postgres::link_as(&state, &app.id, "ledger_prod", Some("PG_URL"))
            .await
            .unwrap();
        let e = create(
            &state,
            &p,
            &db,
            NewRole {
                name: "app".into(),
                env_label: Some("PG_URL".into()),
                connection_limit: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(e.to_string(), "PG_URL is already the label of ledger_prod.");
        assert!(
            p.sql().iter().all(|s| !s.contains("\"ledger_prod_app\"")),
            "nothing reaches the cluster"
        );
        create(&state, &p, &db, new_role("app")).await.unwrap();
        let e = set_labels(&state, &db, &[("app".into(), "PG_URL".into())])
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), "PG_URL is already the label of ledger_prod.");
    }

    #[tokio::test]
    async fn a_manifest_creates_what_it_names_labels_it_and_keeps_what_it_dropped() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = crate::apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let db = ledger(&state, &p).await;
        create(&state, &p, &db, new_role("old")).await.unwrap();
        let mut spec = DatabaseSpec {
            url: Some("DATABASE_ADMIN_URL".into()),
            ..DatabaseSpec::default()
        };
        spec.roles.insert(
            "app".into(),
            RoleSpec {
                url: Some("DATABASE_URL".into()),
            },
        );
        spec.roles.insert("reader".into(), RoleSpec { url: None });
        ensure_from_manifest(&state, &p, &db, &spec, &app)
            .await
            .unwrap();

        let labels: Vec<(String, String)> = list_for(&state, &db.id)
            .await
            .unwrap()
            .into_iter()
            .map(|r| (r.name, r.env_label))
            .collect();
        assert_eq!(
            labels,
            [
                ("ledger_prod".into(), "DATABASE_ADMIN_URL".into()),
                ("ledger_prod_app".into(), "DATABASE_URL".into()),
                ("ledger_prod_old".into(), "DATABASE_URL_OLD".into()),
                ("ledger_prod_reader".into(), "DATABASE_URL_READER".into()),
            ]
        );
        let kept = events::list(&state, 10, true).await.unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].kind, "role_kept");
        assert_eq!(kept[0].subject, "ledger_prod_old");
        assert_eq!(kept[0].app_id.as_deref(), Some(app.id.as_str()));

        let creates = p
            .sql()
            .iter()
            .filter(|s| s.starts_with("CREATE ROLE"))
            .count();
        ensure_from_manifest(&state, &p, &db, &spec, &app)
            .await
            .unwrap();
        assert_eq!(
            p.sql()
                .iter()
                .filter(|s| s.starts_with("CREATE ROLE"))
                .count(),
            creates,
            "a second deploy creates nothing"
        );
        assert_eq!(events::list(&state, 10, true).await.unwrap().len(), 1);

        let mut twice = spec.clone();
        twice.roles.insert(
            "other".into(),
            RoleSpec {
                url: Some("DATABASE_ADMIN_URL".into()),
            },
        );
        let e = ensure_from_manifest(&state, &p, &db, &twice, &app)
            .await
            .unwrap_err();
        assert_eq!(e.to_string(), "ferrum.toml names DATABASE_ADMIN_URL twice.");

        let creates = p
            .sql()
            .iter()
            .filter(|s| s.starts_with("CREATE ROLE"))
            .count();
        let mut taken = spec.clone();
        taken.roles.insert(
            "b".into(),
            RoleSpec {
                url: Some("DATABASE_URL_OLD".into()),
            },
        );
        let e = ensure_from_manifest(&state, &p, &db, &taken, &app)
            .await
            .unwrap_err();
        assert_eq!(
            e.to_string(),
            "DATABASE_URL_OLD is already the label of another role of ledger_prod."
        );
        assert_eq!(
            p.sql()
                .iter()
                .filter(|s| s.starts_with("CREATE ROLE"))
                .count(),
            creates,
            "a label an unlisted role holds is refused before anything is created"
        );
        assert!(find(&state, &db, "b").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn the_owner_bypasses_rls_only_while_the_file_says_so() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = crate::apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let db = ledger(&state, &p).await;
        let alters = |p: &FakePlatform| {
            p.sql()
                .into_iter()
                .filter(|s| s.contains("BYPASSRLS"))
                .collect::<Vec<_>>()
        };

        let plain = DatabaseSpec::default();
        ensure_from_manifest(&state, &p, &db, &plain, &app)
            .await
            .unwrap();
        assert!(
            alters(&p).is_empty(),
            "an owner that never asked is left alone"
        );
        assert!(
            !find(&state, &db, &db.role)
                .await
                .unwrap()
                .unwrap()
                .bypass_rls
        );

        let on = DatabaseSpec {
            bypass_rls: true,
            ..DatabaseSpec::default()
        };
        ensure_from_manifest(&state, &p, &db, &on, &app)
            .await
            .unwrap();
        ensure_from_manifest(&state, &p, &db, &on, &app)
            .await
            .unwrap();
        assert_eq!(alters(&p), ["ALTER ROLE \"ledger_prod\" BYPASSRLS;\n"]);
        assert!(
            find(&state, &db, &db.role)
                .await
                .unwrap()
                .unwrap()
                .bypass_rls
        );

        ensure_from_manifest(&state, &p, &db, &plain, &app)
            .await
            .unwrap();
        assert_eq!(
            alters(&p).last().unwrap(),
            "ALTER ROLE \"ledger_prod\" NOBYPASSRLS;\n"
        );
        assert!(
            !find(&state, &db, &db.role)
                .await
                .unwrap()
                .unwrap()
                .bypass_rls
        );
    }
}
