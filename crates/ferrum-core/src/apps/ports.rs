use anyhow::bail;
use sqlx::{Sqlite, Transaction};
use std::collections::HashSet;
use std::ops::RangeInclusive;

pub const RANGE: RangeInclusive<u16> = 20000..=29999;

/// Reuses the app's existing port for the name, else the lowest port in the range that neither
/// the table nor the host holds.
pub async fn allocate(
    tx: &mut Transaction<'_, Sqlite>,
    app_id: &str,
    name: &str,
    taken_on_host: &(dyn Fn(u16) -> bool + Send + Sync),
) -> anyhow::Result<u16> {
    let existing = sqlx::query_scalar!(
        r#"SELECT port AS "port!" FROM app_ports WHERE app_id = ? AND name = ?"#,
        app_id,
        name
    )
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(port) = existing {
        return Ok(port as u16);
    }

    let taken: HashSet<u16> =
        sqlx::query_scalar!(r#"SELECT port AS "port!" FROM app_ports ORDER BY port"#)
            .fetch_all(&mut **tx)
            .await?
            .into_iter()
            .map(|port| port as u16)
            .collect();
    let mut skipped = Vec::new();
    let found = RANGE.clone().find(|port| {
        if taken.contains(port) {
            return false;
        }
        if taken_on_host(*port) {
            skipped.push(*port);
            return false;
        }
        true
    });
    if !skipped.is_empty() {
        tracing::info!(ports = ?skipped, "in use on the host, passed over");
    }
    let Some(candidate) = found else {
        bail!(
            "every port between {} and {} is reserved or in use",
            RANGE.start(),
            RANGE.end()
        );
    };

    let port = candidate as i64;
    sqlx::query!(
        "INSERT INTO app_ports (port, app_id, name) VALUES (?, ?, ?)",
        port,
        app_id,
        name
    )
    .execute(&mut **tx)
    .await?;
    Ok(candidate)
}

pub async fn release_unused(
    tx: &mut Transaction<'_, Sqlite>,
    app_id: &str,
    kept: &[&str],
) -> anyhow::Result<()> {
    let named = sqlx::query!(
        r#"SELECT name AS "name!" FROM app_ports WHERE app_id = ?"#,
        app_id
    )
    .fetch_all(&mut **tx)
    .await?;
    for row in named {
        if !kept.contains(&row.name.as_str()) {
            sqlx::query!(
                "DELETE FROM app_ports WHERE app_id = ? AND name = ?",
                app_id,
                row.name
            )
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::apps::tests::{new_app, state};
    use crate::apps::{create, delete};

    #[tokio::test]
    async fn the_lowest_free_port_is_handed_out_and_gaps_are_refilled() {
        let (_d, state) = state().await;
        let a = create(&state, new_app("a", &[("/", "main", false)]))
            .await
            .unwrap();
        let b = create(&state, new_app("b", &[("/", "main", false)]))
            .await
            .unwrap();
        assert_eq!(a.main_port(), Some(20000));
        assert_eq!(b.main_port(), Some(20001));

        delete(&state, "a").await.unwrap();
        let c = create(&state, new_app("c", &[("/", "main", false)]))
            .await
            .unwrap();
        assert_eq!(c.main_port(), Some(20000));
    }

    #[tokio::test]
    async fn a_port_something_else_on_the_host_holds_is_passed_over() {
        let (_d, mut state) = state().await;
        state.port_taken = std::sync::Arc::new(|port| port == 20000 || port == 20002);
        let a = create(&state, new_app("a", &[("/", "main", false)]))
            .await
            .unwrap();
        let b = create(&state, new_app("b", &[("/", "main", false)]))
            .await
            .unwrap();
        assert_eq!(a.main_port(), Some(20001));
        assert_eq!(b.main_port(), Some(20003));
    }

    #[tokio::test]
    async fn the_schema_refuses_a_port_outside_the_range_or_held_twice() {
        let (_d, state) = state().await;
        let a = create(&state, new_app("a", &[("/", "main", false)]))
            .await
            .unwrap();
        let outside =
            sqlx::query("INSERT INTO app_ports (port, app_id, name) VALUES (8080, ?, 'x')")
                .bind(&a.id)
                .execute(&state.pool)
                .await;
        assert!(outside.is_err());
        let twice =
            sqlx::query("INSERT INTO app_ports (port, app_id, name) VALUES (20000, ?, 'x')")
                .bind(&a.id)
                .execute(&state.pool)
                .await;
        assert!(twice.is_err());
    }
}
