use super::provision::app_dir;
use super::{App, AppError};
use crate::postgres::DbError;
use crate::secrets::{self, Key};
use crate::state::State;
use crate::{postgres, redis};
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};
use std::collections::BTreeMap;

pub const HOST: &str = "127.0.0.1";
pub const REDIS_URL_KEY: &str = "REDIS_URL";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    Owner { database: String },
    Role { database: String, role: String },
    Redis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedVar {
    pub key: String,
    pub value: String,
    pub origin: Origin,
}

/// A managed variable as the panel sees it: where it comes from, never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Label {
    pub key: String,
    #[serde(flatten)]
    pub origin: Origin,
}

/// Variables Ferrum owns: rendered from links at write time, never stored, so a relink cannot
/// leave a stale copy behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Managed {
    pub vars: Vec<ManagedVar>,
}

impl Managed {
    pub fn keys(&self) -> Vec<String> {
        self.vars.iter().map(|v| v.key.clone()).collect()
    }

    pub fn pairs(&self) -> Vec<(String, String)> {
        self.vars
            .iter()
            .map(|v| (v.key.clone(), v.value.clone()))
            .collect()
    }

    pub fn labels(&self) -> Vec<Label> {
        self.vars
            .iter()
            .map(|v| Label {
                key: v.key.clone(),
                origin: v.origin.clone(),
            })
            .collect()
    }
}

pub async fn managed_for(state: &State, app: &App) -> anyhow::Result<Managed> {
    let mut vars = postgres::urls_for(state, &app.id).await?;
    if let Some((key, value)) = redis::url_for(state, &app.id).await? {
        vars.push(ManagedVar {
            key,
            value,
            origin: Origin::Redis,
        });
    }
    Ok(Managed { vars })
}

/// What the panel sends to rename managed variables; roles are keyed `<database>/<role>`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LabelChanges {
    pub database: BTreeMap<String, String>,
    pub roles: BTreeMap<String, String>,
    pub redis: Option<String>,
}

pub const FOLLOWS_FILE: &str =
    "The repo's file names the labels; turn following off to edit them here.";

pub async fn set_labels(state: &State, app: &App, changes: &LabelChanges) -> anyhow::Result<()> {
    if app.follow_repo_file {
        return Err(DbError::Conflict(FOLLOWS_FILE.into()).into());
    }
    let mut seen: Vec<&String> = Vec::new();
    for label in changes
        .database
        .values()
        .chain(changes.roles.values())
        .chain(changes.redis.iter())
    {
        postgres::roles::valid_label(label)?;
        if seen.contains(&label) {
            return Err(DbError::Invalid(format!("{label} is named twice.")).into());
        }
        seen.push(label);
    }
    let linked = postgres::linked_to(state, &app.id).await?;
    let linked_db = |name: &str| {
        linked
            .iter()
            .find(|d| d.name == name)
            .ok_or_else(|| DbError::Missing(format!("{name} is not linked to {}.", app.slug)))
    };
    let mut per_db: BTreeMap<&str, Vec<(String, String)>> = BTreeMap::new();
    for (name, label) in &changes.database {
        let db = linked_db(name)?;
        per_db
            .entry(db.name.as_str())
            .or_default()
            .push((db.role.clone(), label.clone()));
    }
    for (key, label) in &changes.roles {
        let (name, role) = key
            .split_once('/')
            .ok_or_else(|| DbError::Invalid(format!("{key} is not <database>/<role>.")))?;
        let db = linked_db(name)?;
        let found = postgres::roles::find(state, db, role)
            .await?
            .filter(|r| !r.owner)
            .ok_or_else(|| DbError::Missing(format!("{name} has no role called {role}.")))?;
        per_db
            .entry(db.name.as_str())
            .or_default()
            .push((found.name, label.clone()));
    }
    if changes.redis.is_some() && redis::for_app(state, &app.id).await?.is_none() {
        return Err(DbError::Missing(format!("{} has no Redis instance.", app.slug)).into());
    }

    for (name, labels) in &per_db {
        postgres::roles::set_labels(state, linked_db(name)?, labels).await?;
    }
    for (name, label) in &changes.database {
        postgres::set_link_label(state, &app.id, name, Some(label)).await?;
    }
    if let Some(label) = &changes.redis {
        redis::set_label(state, &app.id, label).await?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    pub key: String,
    pub value: String,
}

pub fn valid_key(key: &str) -> Result<(), AppError> {
    let mut chars = key.chars();
    let ok = matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(AppError::Invalid(format!(
            "{key:?} is not a valid variable name; use letters, digits and underscores."
        )))
    }
}

/// Every process with a port is named in the shared env as `<NAME>_PORT`; `PORT` itself is set
/// per unit, so each program sees only its own.
pub fn port_var(name: &str) -> String {
    format!("{}_PORT", name.to_ascii_uppercase())
}

/// The variables Ferrum sets that a user variable may never shadow.
pub fn reserved_keys(ports: &[(String, u16)]) -> Vec<String> {
    let mut keys = vec!["PORT".to_string(), "HOST".to_string()];
    keys.extend(ports.iter().map(|(name, _)| port_var(name)));
    keys
}

pub async fn set(state: &State, app_id: &str, key: &str, value: &str) -> anyhow::Result<()> {
    let mut tx = state.pool.begin().await?;
    set_in(&mut tx, &state.key, app_id, key, value).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_in(
    tx: &mut Transaction<'_, Sqlite>,
    secret: &Key,
    app_id: &str,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    valid_key(key)?;
    let sealed = secrets::encrypt(secret, value);
    sqlx::query!(
        "INSERT INTO app_env (app_id, key, value) VALUES (?, ?, ?)
         ON CONFLICT(app_id, key) DO UPDATE SET value = excluded.value",
        app_id,
        key,
        sealed
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub async fn remove(state: &State, app_id: &str, key: &str) -> anyhow::Result<bool> {
    let done = sqlx::query!(
        "DELETE FROM app_env WHERE app_id = ? AND key = ?",
        app_id,
        key
    )
    .execute(&state.pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// A row without a value keeps the value already stored, so the panel never has to read one back.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EnvChange {
    pub key: String,
    #[serde(default)]
    pub value: Option<String>,
}

pub async fn replace(state: &State, app_id: &str, vars: &[EnvChange]) -> anyhow::Result<()> {
    for var in vars {
        valid_key(&var.key)?;
    }
    let existing = all(state, app_id).await?;
    let mut tx = state.pool.begin().await?;
    for (key, _) in &existing {
        if !vars.iter().any(|v| &v.key == key) {
            sqlx::query!(
                "DELETE FROM app_env WHERE app_id = ? AND key = ?",
                app_id,
                key
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    for var in vars {
        match &var.value {
            Some(value) => set_in(&mut tx, &state.key, app_id, &var.key, value).await?,
            None if existing.iter().any(|(k, _)| k == &var.key) => {}
            None => {
                return Err(AppError::Invalid(format!("{} has no value yet.", var.key)).into());
            }
        }
    }
    tx.commit().await?;
    Ok(())
}

pub async fn all(state: &State, app_id: &str) -> anyhow::Result<Vec<(String, String)>> {
    let rows = sqlx::query!(
        r#"SELECT key AS "key!", value AS "value!" FROM app_env WHERE app_id = ? ORDER BY key"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    rows.into_iter()
        .map(|r| Ok((r.key, secrets::decrypt(&state.key, &r.value)?)))
        .collect()
}

pub async fn keys(state: &State, app_id: &str) -> anyhow::Result<Vec<String>> {
    Ok(all(state, app_id)
        .await?
        .into_iter()
        .map(|(k, _)| k)
        .collect())
}

pub const FILE_SOURCE: &str = "ferrum.toml";
const SHARED_PLACEHOLDER: &str = "{{shared}}";

/// A variable the repository says it reads, with a sentence for the panel and a non-secret
/// default; `{{shared}}` in a default stands for the app's shared directory.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvRequirement {
    pub key: String,
    #[serde(default)]
    pub about: Option<String>,
    #[serde(default)]
    pub default: Option<String>,
}

pub fn expand_default(value: &str, slug: &str) -> String {
    value.replace(
        SHARED_PLACEHOLDER,
        &app_dir(slug).join("shared").to_string_lossy(),
    )
}

/// What the panel shows per key: values never leave the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub key: String,
    pub set: bool,
    pub source: Option<String>,
    pub about: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Required {
    pub key: String,
    pub source: String,
    pub about: Option<String>,
    pub default: Option<String>,
}

pub async fn required(state: &State, app_id: &str) -> anyhow::Result<Vec<Required>> {
    let rows = sqlx::query!(
        r#"SELECT key AS "key!", source AS "source!", about, default_value
           FROM app_env_required WHERE app_id = ? ORDER BY rowid"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Required {
            key: r.key,
            source: r.source,
            about: r.about,
            default: r.default_value,
        })
        .collect())
}

/// The file's list replaces the file's list; a key a failed command named stays until a value
/// exists or the file claims it.
pub async fn replace_required(
    tx: &mut Transaction<'_, Sqlite>,
    app_id: &str,
    required: &[EnvRequirement],
) -> anyhow::Result<()> {
    sqlx::query!(
        "DELETE FROM app_env_required WHERE app_id = ? AND source = ?",
        app_id,
        FILE_SOURCE
    )
    .execute(&mut **tx)
    .await?;
    for req in required {
        valid_key(&req.key)?;
        sqlx::query!(
            "INSERT OR REPLACE INTO app_env_required (app_id, key, source, about, default_value)
             VALUES (?, ?, ?, ?, ?)",
            app_id,
            req.key,
            FILE_SOURCE,
            req.about,
            req.default
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// A key a failed command complained about shows as missing until a value exists.
pub async fn note_named_by_failure(
    state: &State,
    app_id: &str,
    key: &str,
    what: &str,
) -> anyhow::Result<()> {
    valid_key(key)?;
    let source = format!("named by the failed {what}");
    sqlx::query!(
        "INSERT OR IGNORE INTO app_env_required (app_id, key, source) VALUES (?, ?, ?)",
        app_id,
        key,
        source
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Stored keys first, then the required keys nothing has set yet.
pub async fn entries(state: &State, app_id: &str) -> anyhow::Result<Vec<Entry>> {
    let stored = keys(state, app_id).await?;
    let required = required(state, app_id).await?;
    let mut entries: Vec<Entry> = stored
        .iter()
        .map(|key| {
            let req = required.iter().find(|r| &r.key == key);
            Entry {
                key: key.clone(),
                set: true,
                source: req.map(|r| r.source.clone()),
                about: req.and_then(|r| r.about.clone()),
            }
        })
        .collect();
    entries.extend(
        required
            .into_iter()
            .filter(|r| !stored.contains(&r.key))
            .map(|r| Entry {
                key: r.key,
                set: false,
                source: Some(r.source),
                about: r.about,
            }),
    );
    Ok(entries)
}

/// Everything the env file carries, in its order. A managed or reserved key wins over a user
/// variable of the same name. `ports` is each port process with its port.
pub fn pairs(
    vars: &[(String, String)],
    managed: &Managed,
    ports: &[(String, u16)],
) -> Vec<(String, String)> {
    let managed = managed.pairs();
    let reserved = reserved_keys(ports);
    let mut out: Vec<(String, String)> = vars
        .iter()
        .filter(|(key, _)| !managed.iter().any(|(m, _)| m == key) && !reserved.contains(key))
        .chain(managed.iter())
        .cloned()
        .collect();
    for (name, port) in ports {
        out.push((port_var(name), port.to_string()));
    }
    out.push(("HOST".into(), HOST.into()));
    out
}

/// systemd's `EnvironmentFile=` dialect: no expansion, but an unquoted backslash is an escape.
pub fn render(vars: &[(String, String)], managed: &Managed, ports: &[(String, u16)]) -> String {
    let mut out = String::new();
    for (key, value) in pairs(vars, managed, ports) {
        out.push_str(&key);
        out.push('=');
        out.push_str(&quote(&value));
        out.push('\n');
    }
    out
}

fn quote(value: &str) -> String {
    let needs = value.is_empty()
        || value
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '#' | '"' | '\'' | '\\' | ';'));
    if !needs {
        return value.to_string();
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for c in value.chars() {
        if c == '"' || c == '\\' {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};

    fn ports(list: &[(&str, u16)]) -> Vec<(String, u16)> {
        list.iter().map(|(n, p)| (n.to_string(), *p)).collect()
    }

    #[test]
    fn env_renders_user_vars_then_every_process_port_and_quotes_nothing_it_does_not_have_to() {
        let out = render(
            &[
                ("DATABASE_URL".into(), "postgres://x".into()),
                ("GREETING".into(), "hello world".into()),
            ],
            &Managed::default(),
            &ports(&[("web", 20000), ("ws", 20001)]),
        );
        assert_eq!(
            out,
            "DATABASE_URL=postgres://x\nGREETING=\"hello world\"\nWEB_PORT=20000\nWS_PORT=20001\nHOST=127.0.0.1\n"
        );
    }

    #[test]
    fn a_user_variable_cannot_shadow_a_port_or_the_host() {
        let out = render(
            &[
                ("PORT".into(), "80".into()),
                ("WEB_PORT".into(), "81".into()),
                ("HOST".into(), "0.0.0.0".into()),
                ("KEEP".into(), "1".into()),
            ],
            &Managed::default(),
            &ports(&[("web", 20000)]),
        );
        assert_eq!(out, "KEEP=1\nWEB_PORT=20000\nHOST=127.0.0.1\n");
    }

    fn owner(key: &str, value: &str) -> ManagedVar {
        ManagedVar {
            key: key.into(),
            value: value.into(),
            origin: Origin::Owner {
                database: "ledger_prod".into(),
            },
        }
    }

    #[test]
    fn managed_variables_come_after_the_users_and_before_the_ports() {
        let managed = Managed {
            vars: vec![
                owner("DATABASE_URL", "postgres://a:b@127.0.0.1:5432/ledger_prod"),
                ManagedVar {
                    key: REDIS_URL_KEY.into(),
                    value: "redis://:pw@127.0.0.1:20001/0".into(),
                    origin: Origin::Redis,
                },
            ],
        };
        let out = render(
            &[("APP_KEY".into(), "x".into())],
            &managed,
            &ports(&[("web", 20000)]),
        );
        assert_eq!(
            out,
            "APP_KEY=x\nDATABASE_URL=postgres://a:b@127.0.0.1:5432/ledger_prod\nREDIS_URL=redis://:pw@127.0.0.1:20001/0\nWEB_PORT=20000\nHOST=127.0.0.1\n"
        );
        assert_eq!(managed.keys(), vec!["DATABASE_URL", "REDIS_URL"]);
    }

    #[test]
    fn a_user_variable_named_database_url_is_overridden_by_the_link_not_duplicated() {
        let managed = Managed {
            vars: vec![owner("DATABASE_URL", "postgres://real")],
        };
        let out = render(
            &[("DATABASE_URL".into(), "postgres://stale".into())],
            &managed,
            &[],
        );
        assert_eq!(out.matches("DATABASE_URL=").count(), 1);
        assert!(out.contains("DATABASE_URL=postgres://real\n"));
    }

    #[tokio::test]
    async fn managed_variables_follow_the_links_and_the_redis_instance() {
        let (_d, state) = state().await;
        let p = ferrum_platform::FakePlatform::new();
        let app = crate::apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        assert_eq!(managed_for(&state, &app).await.unwrap(), Managed::default());
        postgres::create(&state, &p, postgres::tests::new("ledger_prod"))
            .await
            .unwrap();
        postgres::create(&state, &p, postgres::tests::new("analytics"))
            .await
            .unwrap();
        postgres::link(&state, &app.id, "ledger_prod")
            .await
            .unwrap();
        postgres::link(&state, &app.id, "analytics").await.unwrap();
        p.set_active("ferrum-redis-ledger");
        let instance = redis::request(&state, &p, &app, 64).await.unwrap();
        let managed = managed_for(&state, &app).await.unwrap();
        assert_eq!(
            managed.keys(),
            vec!["DATABASE_URL", "ANALYTICS_DATABASE_URL", "REDIS_URL"]
        );
        let redis = managed.vars.last().unwrap();
        assert_eq!(redis.origin, Origin::Redis);
        assert!(
            redis
                .value
                .ends_with(&format!("@127.0.0.1:{}/0", instance.port))
        );
    }

    #[tokio::test]
    async fn labels_rename_the_owner_each_role_and_redis_unless_the_file_names_them() {
        let (_d, state) = state().await;
        let p = ferrum_platform::FakePlatform::new();
        let mut app = crate::apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let db = postgres::create(&state, &p, postgres::tests::new("ledger_prod"))
            .await
            .unwrap();
        postgres::roles::create(
            &state,
            &p,
            &db,
            postgres::NewRole {
                name: "app".into(),
                ..postgres::NewRole::default()
            },
        )
        .await
        .unwrap();
        postgres::link(&state, &app.id, "ledger_prod")
            .await
            .unwrap();
        p.set_active("ferrum-redis-ledger");
        redis::request(&state, &p, &app, 64).await.unwrap();
        assert_eq!(
            managed_for(&state, &app).await.unwrap().keys(),
            ["DATABASE_URL", "DATABASE_URL_APP", "REDIS_URL"]
        );

        let mut changes = LabelChanges {
            redis: Some("CACHE_URL".into()),
            ..LabelChanges::default()
        };
        changes
            .database
            .insert("ledger_prod".into(), "DATABASE_ADMIN_URL".into());
        changes
            .roles
            .insert("ledger_prod/app".into(), "DATABASE_URL".into());
        set_labels(&state, &app, &changes).await.unwrap();
        let managed = managed_for(&state, &app).await.unwrap();
        assert_eq!(
            managed.keys(),
            ["DATABASE_ADMIN_URL", "DATABASE_URL", "CACHE_URL"]
        );
        assert_eq!(
            serde_json::to_value(managed.labels()).unwrap(),
            serde_json::json!([
                {"key": "DATABASE_ADMIN_URL", "kind": "owner", "database": "ledger_prod"},
                {"key": "DATABASE_URL", "kind": "role", "database": "ledger_prod", "role": "ledger_prod_app"},
                {"key": "CACHE_URL", "kind": "redis"}
            ])
        );

        let mut twice = LabelChanges::default();
        twice.database.insert("ledger_prod".into(), "X_URL".into());
        twice.roles.insert("ledger_prod/app".into(), "X_URL".into());
        assert_eq!(
            set_labels(&state, &app, &twice)
                .await
                .unwrap_err()
                .to_string(),
            "X_URL is named twice."
        );
        let mut stranger = LabelChanges::default();
        stranger.database.insert("analytics".into(), "X_URL".into());
        assert!(set_labels(&state, &app, &stranger).await.is_err());

        app.follow_repo_file = true;
        assert_eq!(
            set_labels(&state, &app, &changes)
                .await
                .unwrap_err()
                .to_string(),
            FOLLOWS_FILE
        );
    }

    #[test]
    fn values_systemd_would_mangle_are_quoted_and_escaped() {
        assert_eq!(quote("a\\b"), "\"a\\\\b\"");
        assert_eq!(quote("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote("x#y"), "\"x#y\"");
        assert_eq!(quote("it's"), "\"it's\"");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(
            quote("$HOME/x"),
            "$HOME/x",
            "a dollar is literal and needs nothing"
        );
        assert_eq!(
            quote("postgres://u:p@h/db?sslmode=require"),
            "postgres://u:p@h/db?sslmode=require"
        );
    }

    #[test]
    fn env_keys_that_are_not_identifiers_are_refused() {
        for bad in ["1ABC", "A-B", "A B", "", "A=B", "PATH "] {
            assert!(valid_key(bad).is_err(), "{bad:?}");
        }
        for good in ["A", "_x", "DATABASE_URL", "NEXT_PUBLIC_API_1"] {
            assert!(valid_key(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn process_names_become_uppercase_port_variables() {
        assert_eq!(port_var("web"), "WEB_PORT");
        assert_eq!(port_var("ws"), "WS_PORT");
        assert_eq!(port_var("admin_ui"), "ADMIN_UI_PORT");
        assert_eq!(
            reserved_keys(&[("web".into(), 1)]),
            vec!["PORT", "HOST", "WEB_PORT"]
        );
    }

    #[tokio::test]
    async fn variables_round_trip_and_replace_wholesale() {
        let (_d, state) = state().await;
        let app = crate::apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        set(&state, &app.id, "B", "2").await.unwrap();
        set(&state, &app.id, "A", "1").await.unwrap();
        set(&state, &app.id, "A", "one").await.unwrap();
        assert_eq!(
            all(&state, &app.id).await.unwrap(),
            vec![("A".into(), "one".into()), ("B".into(), "2".into())]
        );
        assert!(remove(&state, &app.id, "B").await.unwrap());
        assert!(!remove(&state, &app.id, "B").await.unwrap());

        replace(
            &state,
            &app.id,
            &[EnvChange {
                key: "ONLY".into(),
                value: Some("this".into()),
            }],
        )
        .await
        .unwrap();
        assert_eq!(keys(&state, &app.id).await.unwrap(), vec!["ONLY"]);

        replace(
            &state,
            &app.id,
            &[
                EnvChange {
                    key: "ONLY".into(),
                    value: None,
                },
                EnvChange {
                    key: "NEW".into(),
                    value: Some("n".into()),
                },
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            all(&state, &app.id).await.unwrap(),
            vec![("NEW".into(), "n".into()), ("ONLY".into(), "this".into())],
            "a row without a value keeps what was stored"
        );
        let unknown = replace(
            &state,
            &app.id,
            &[EnvChange {
                key: "GHOST".into(),
                value: None,
            }],
        )
        .await;
        assert!(unknown.is_err(), "a new key needs a value");

        assert!(set(&state, &app.id, "1BAD", "x").await.is_err());
        let forced = sqlx::query("INSERT INTO app_env (app_id, key, value) VALUES (?, 'A-B', 'x')")
            .bind(&app.id)
            .execute(&state.pool)
            .await;
        assert!(
            forced.is_err(),
            "the schema refuses a key the shell would refuse"
        );
    }
}
