use super::{App, AppError, Route};
pub use crate::detect::env_hints::EnvHint;
use crate::secrets::{self, Key};
use crate::state::State;
use crate::{postgres, redis};
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

pub const HOST: &str = "127.0.0.1";
pub const REDIS_URL_KEY: &str = "REDIS_URL";

/// Variables Ferrum owns: rendered from links at write time, never stored, so a relink cannot
/// leave a stale copy behind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Managed {
    pub database_urls: Vec<(String, String)>,
    pub redis_url: Option<String>,
}

impl Managed {
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.database_urls.iter().map(|(k, _)| k.clone()).collect();
        if self.redis_url.is_some() {
            keys.push(REDIS_URL_KEY.to_string());
        }
        keys
    }

    fn pairs(&self) -> Vec<(String, String)> {
        let mut pairs = self.database_urls.clone();
        if let Some(url) = &self.redis_url {
            pairs.push((REDIS_URL_KEY.to_string(), url.clone()));
        }
        pairs
    }
}

pub async fn managed_for(state: &State, app: &App) -> anyhow::Result<Managed> {
    Ok(Managed {
        database_urls: postgres::urls_for(state, &app.id).await?,
        redis_url: redis::url_for(state, &app.id).await?,
    })
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

pub fn port_var(name: &str) -> String {
    if name == "main" {
        "PORT".to_string()
    } else {
        format!("{}_PORT", name.to_ascii_uppercase())
    }
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

/// What the panel shows per key: hints come from the repository, values never leave the server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub key: String,
    pub set: bool,
    pub source: Option<String>,
    pub optional: bool,
}

pub async fn hints(state: &State, app_id: &str) -> anyhow::Result<Vec<EnvHint>> {
    let rows = sqlx::query!(
        r#"SELECT key AS "key!", source AS "source!", optional AS "optional!: bool"
           FROM app_env_hints WHERE app_id = ? ORDER BY rowid"#,
        app_id
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| EnvHint {
            key: r.key,
            source: r.source,
            optional: r.optional,
            suggest_app_url: false,
        })
        .collect())
}

pub async fn replace_hints(
    tx: &mut Transaction<'_, Sqlite>,
    app_id: &str,
    hints: &[EnvHint],
) -> anyhow::Result<()> {
    sqlx::query!("DELETE FROM app_env_hints WHERE app_id = ?", app_id)
        .execute(&mut **tx)
        .await?;
    for hint in hints {
        valid_key(&hint.key)?;
        sqlx::query!(
            "INSERT INTO app_env_hints (app_id, key, source, optional) VALUES (?, ?, ?, ?)",
            app_id,
            hint.key,
            hint.source,
            hint.optional
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

/// Adds a hint for a key that has none yet; a creation-time source is kept over a later one.
pub async fn add_hint(
    state: &State,
    app_id: &str,
    key: &str,
    source: &str,
    optional: bool,
) -> anyhow::Result<()> {
    valid_key(key)?;
    sqlx::query!(
        "INSERT OR IGNORE INTO app_env_hints (app_id, key, source, optional) VALUES (?, ?, ?, ?)",
        app_id,
        key,
        source,
        optional
    )
    .execute(&state.pool)
    .await?;
    Ok(())
}

/// Stored keys first, then the hinted keys nothing has set yet.
pub async fn entries(state: &State, app_id: &str) -> anyhow::Result<Vec<Entry>> {
    let stored = keys(state, app_id).await?;
    let hints = hints(state, app_id).await?;
    let mut entries: Vec<Entry> = stored
        .iter()
        .map(|key| {
            let hint = hints.iter().find(|h| &h.key == key);
            Entry {
                key: key.clone(),
                set: true,
                source: hint.map(|h| h.source.clone()),
                optional: hint.is_some_and(|h| h.optional),
            }
        })
        .collect();
    entries.extend(
        hints
            .into_iter()
            .filter(|h| !stored.contains(&h.key))
            .map(|h| Entry {
                key: h.key,
                set: false,
                source: Some(h.source),
                optional: h.optional,
            }),
    );
    Ok(entries)
}

/// Everything the env file carries, in its order. A managed key wins over a user variable of
/// the same name.
pub fn pairs(
    vars: &[(String, String)],
    managed: &Managed,
    routes: &[Route],
) -> Vec<(String, String)> {
    let managed = managed.pairs();
    let mut out: Vec<(String, String)> = vars
        .iter()
        .filter(|(key, _)| !managed.iter().any(|(m, _)| m == key))
        .chain(managed.iter())
        .cloned()
        .collect();
    let mut seen = Vec::new();
    for route in routes {
        if seen.contains(&route.port_name) {
            continue;
        }
        seen.push(route.port_name.clone());
        out.push((port_var(&route.port_name), route.port.to_string()));
    }
    out.push(("HOST".into(), HOST.into()));
    out
}

/// systemd's `EnvironmentFile=` dialect: no expansion, but an unquoted backslash is an escape.
pub fn render(vars: &[(String, String)], managed: &Managed, routes: &[Route]) -> String {
    let mut out = String::new();
    for (key, value) in pairs(vars, managed, routes) {
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
    use crate::apps::tests::{new_app, route, state};

    #[test]
    fn env_renders_user_vars_then_ports_and_quotes_nothing_it_does_not_have_to() {
        let routes = vec![
            route("/", "main", 20000, false),
            route("/ws", "ws", 20001, true),
        ];
        let out = render(
            &[
                ("DATABASE_URL".into(), "postgres://x".into()),
                ("GREETING".into(), "hello world".into()),
            ],
            &Managed::default(),
            &routes,
        );
        assert_eq!(
            out,
            "DATABASE_URL=postgres://x\nGREETING=\"hello world\"\nPORT=20000\nWS_PORT=20001\nHOST=127.0.0.1\n"
        );
    }

    #[test]
    fn a_shared_port_name_is_written_once() {
        let routes = vec![
            route("/", "main", 20000, false),
            route("/api", "main", 20000, false),
        ];
        assert_eq!(
            render(&[], &Managed::default(), &routes),
            "PORT=20000\nHOST=127.0.0.1\n"
        );
    }

    #[test]
    fn managed_variables_come_after_the_users_and_before_the_ports() {
        let managed = Managed {
            database_urls: vec![(
                "DATABASE_URL".into(),
                "postgres://a:b@127.0.0.1:5432/ledger_prod".into(),
            )],
            redis_url: Some("redis://:pw@127.0.0.1:20001/0".into()),
        };
        let out = render(
            &[("APP_KEY".into(), "x".into())],
            &managed,
            &[route("/", "main", 20000, false)],
        );
        assert_eq!(
            out,
            "APP_KEY=x\nDATABASE_URL=postgres://a:b@127.0.0.1:5432/ledger_prod\nREDIS_URL=redis://:pw@127.0.0.1:20001/0\nPORT=20000\nHOST=127.0.0.1\n"
        );
        assert_eq!(managed.keys(), vec!["DATABASE_URL", "REDIS_URL"]);
    }

    #[test]
    fn a_user_variable_named_database_url_is_overridden_by_the_link_not_duplicated() {
        let managed = Managed {
            database_urls: vec![("DATABASE_URL".into(), "postgres://real".into())],
            redis_url: None,
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
        let instance = redis::request(&state, &p, &app, 64).await.unwrap();
        let managed = managed_for(&state, &app).await.unwrap();
        assert_eq!(
            managed.keys(),
            vec!["DATABASE_URL", "ANALYTICS_DATABASE_URL", "REDIS_URL"]
        );
        assert!(
            managed
                .redis_url
                .unwrap()
                .ends_with(&format!("@127.0.0.1:{}/0", instance.port))
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
    fn port_names_become_uppercase_variables() {
        assert_eq!(port_var("main"), "PORT");
        assert_eq!(port_var("ws"), "WS_PORT");
        assert_eq!(port_var("admin_ui"), "ADMIN_UI_PORT");
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
