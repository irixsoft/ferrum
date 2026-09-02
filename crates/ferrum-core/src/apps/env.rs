use super::{AppError, Route};
use crate::state::State;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};

pub const HOST: &str = "127.0.0.1";

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
    set_in(&mut tx, app_id, key, value).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_in(
    tx: &mut Transaction<'_, Sqlite>,
    app_id: &str,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    valid_key(key)?;
    sqlx::query!(
        "INSERT INTO app_env (app_id, key, value) VALUES (?, ?, ?)
         ON CONFLICT(app_id, key) DO UPDATE SET value = excluded.value",
        app_id,
        key,
        value
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

pub async fn replace(state: &State, app_id: &str, vars: &[EnvVar]) -> anyhow::Result<()> {
    for var in vars {
        valid_key(&var.key)?;
    }
    let mut tx = state.pool.begin().await?;
    sqlx::query!("DELETE FROM app_env WHERE app_id = ?", app_id)
        .execute(&mut *tx)
        .await?;
    for var in vars {
        set_in(&mut tx, app_id, &var.key, &var.value).await?;
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
    Ok(rows.into_iter().map(|r| (r.key, r.value)).collect())
}

pub async fn keys(state: &State, app_id: &str) -> anyhow::Result<Vec<String>> {
    Ok(all(state, app_id)
        .await?
        .into_iter()
        .map(|(k, _)| k)
        .collect())
}

/// systemd's `EnvironmentFile=` dialect: no expansion, but an unquoted backslash is an escape.
pub fn render(vars: &[(String, String)], routes: &[Route]) -> String {
    let mut out = String::new();
    for (key, value) in vars {
        out.push_str(key);
        out.push('=');
        out.push_str(&quote(value));
        out.push('\n');
    }
    let mut seen = Vec::new();
    for route in routes {
        if seen.contains(&route.port_name) {
            continue;
        }
        seen.push(route.port_name.clone());
        out.push_str(&format!("{}={}\n", port_var(&route.port_name), route.port));
    }
    out.push_str(&format!("HOST={HOST}\n"));
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
        assert_eq!(render(&[], &routes), "PORT=20000\nHOST=127.0.0.1\n");
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
            &[EnvVar {
                key: "ONLY".into(),
                value: "this".into(),
            }],
        )
        .await
        .unwrap();
        assert_eq!(keys(&state, &app.id).await.unwrap(), vec!["ONLY"]);

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
