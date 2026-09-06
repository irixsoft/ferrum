pub mod install;

pub use install::{ensure_installed, installed};

use crate::apps::provision::user_name;
use crate::apps::{App, ports};
use crate::state::State;
use crate::{REDIS_DIR, secret, secrets, time};
use ferrum_platform::ubuntu::{REDIS_SERVER, SYSTEMD_UNIT_DIR};
use ferrum_platform::{Platform, ServiceAction};
use serde::Serialize;
use std::path::{Path, PathBuf};

pub const PORT_NAME: &str = "redis";
pub const DEFAULT_MAXMEMORY_MB: u32 = 64;
pub const MAXMEMORY_RANGE: std::ops::RangeInclusive<u32> = 16..=16_384;

#[derive(Debug, thiserror::Error)]
pub enum RedisError {
    #[error("{0} already has a Redis instance.")]
    Exists(String),
    #[error("Redis memory must be between 16 MB and 16 GB.")]
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
pub struct Instance {
    pub app_id: String,
    pub port: u16,
    pub maxmemory_mb: u32,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Listed {
    pub app_slug: String,
    pub port: u16,
    pub maxmemory_mb: u32,
    pub created_at: String,
}

pub fn unit_name(slug: &str) -> String {
    format!("ferrum-redis-{slug}")
}

pub fn unit_path(slug: &str) -> PathBuf {
    Path::new(SYSTEMD_UNIT_DIR).join(format!("{}.service", unit_name(slug)))
}

pub fn dir(slug: &str) -> PathBuf {
    Path::new(REDIS_DIR).join(slug)
}

pub fn conf_path(slug: &str) -> PathBuf {
    dir(slug).join("redis.conf")
}

pub fn url(port: u16, password: &str) -> String {
    format!("redis://:{password}@127.0.0.1:{port}/0")
}

pub fn render_conf(slug: &str, port: u16, password: &str, maxmemory_mb: u32) -> String {
    let dir = dir(slug);
    let mut c = String::new();
    c.push_str("bind 127.0.0.1\n");
    c.push_str(&format!("port {port}\n"));
    c.push_str("protected-mode yes\n");
    c.push_str("daemonize no\n");
    c.push_str("supervised systemd\n");
    c.push_str(&format!("dir {}\n", dir.display()));
    c.push_str("logfile \"\"\n");
    c.push_str(&format!("requirepass {password}\n"));
    c.push_str(&format!("maxmemory {maxmemory_mb}mb\n"));
    c.push_str("maxmemory-policy noeviction\n");
    c.push_str("appendonly yes\n");
    c.push_str("appendonlydir appendonlydir\n");
    c.push_str("save \"\"\n");
    c
}

pub fn render_unit(slug: &str) -> String {
    let user = user_name(slug);
    let dir = dir(slug);
    let mut u = String::new();
    u.push_str("[Unit]\n");
    u.push_str(&format!("Description=Ferrum redis for {slug}\n"));
    u.push_str("After=network.target\n\n");
    u.push_str("[Service]\n");
    u.push_str("Type=notify\n");
    u.push_str(&format!("User={user}\nGroup={user}\n"));
    u.push_str(&format!(
        "ExecStart={REDIS_SERVER} {}\n",
        conf_path(slug).display()
    ));
    u.push_str("Restart=always\nRestartSec=2\n");
    u.push_str("TimeoutStopSec=30\n");
    u.push_str("NoNewPrivileges=yes\nProtectSystem=strict\nProtectHome=yes\nPrivateTmp=yes\n");
    u.push_str(&format!("ReadWritePaths={}\n", dir.display()));
    u.push_str(&format!("SyslogIdentifier={}\n\n", unit_name(slug)));
    u.push_str("[Install]\nWantedBy=multi-user.target\n");
    u
}

pub async fn for_app(state: &State, app_id: &str) -> anyhow::Result<Option<Instance>> {
    let row = sqlx::query!(
        r#"SELECT r.app_id AS "app_id!", r.maxmemory_mb AS "maxmemory_mb!", r.created_at AS "created_at!",
                  p.port AS "port!"
           FROM redis_instances r JOIN app_ports p ON p.app_id = r.app_id AND p.name = ?
           WHERE r.app_id = ?"#,
        PORT_NAME,
        app_id
    )
    .fetch_optional(&state.pool)
    .await?;
    Ok(row.map(|r| Instance {
        app_id: r.app_id,
        port: r.port as u16,
        maxmemory_mb: r.maxmemory_mb as u32,
        created_at: time::utc(r.created_at),
    }))
}

pub async fn url_for(state: &State, app_id: &str) -> anyhow::Result<Option<String>> {
    let row = sqlx::query!(
        r#"SELECT r.password AS "password!", p.port AS "port!"
           FROM redis_instances r JOIN app_ports p ON p.app_id = r.app_id AND p.name = ?
           WHERE r.app_id = ?"#,
        PORT_NAME,
        app_id
    )
    .fetch_optional(&state.pool)
    .await?;
    row.map(|r| {
        let password = secrets::decrypt(&state.key, &r.password)?;
        Ok(url(r.port as u16, &password))
    })
    .transpose()
}

pub async fn list(state: &State) -> anyhow::Result<Vec<Listed>> {
    let rows = sqlx::query!(
        r#"SELECT a.slug AS "app_slug!", r.maxmemory_mb AS "maxmemory_mb!", r.created_at AS "created_at!",
                  p.port AS "port!"
           FROM redis_instances r
           JOIN apps a ON a.id = r.app_id
           JOIN app_ports p ON p.app_id = r.app_id AND p.name = ?
           ORDER BY a.slug"#,
        PORT_NAME
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Listed {
            app_slug: r.app_slug,
            port: r.port as u16,
            maxmemory_mb: r.maxmemory_mb as u32,
            created_at: time::utc(r.created_at),
        })
        .collect())
}

pub async fn request(
    state: &State,
    platform: &dyn Platform,
    app: &App,
    maxmemory_mb: u32,
) -> anyhow::Result<Instance> {
    if !MAXMEMORY_RANGE.contains(&maxmemory_mb) {
        return Err(RedisError::Invalid.into());
    }
    if for_app(state, &app.id).await?.is_some() {
        return Err(RedisError::Exists(app.slug.clone()).into());
    }
    let password = secret::generate();
    let sealed = secrets::encrypt(&state.key, &password);
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let port = ports::allocate(&mut tx, &app.id, PORT_NAME).await?;
    let maxmemory = maxmemory_mb as i64;
    sqlx::query!(
        "INSERT INTO redis_instances (app_id, password, maxmemory_mb) VALUES (?, ?, ?)",
        app.id,
        sealed,
        maxmemory
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    if let Err(e) = provision(platform, &app.slug, port, &password, maxmemory_mb) {
        remove_from_host(platform, &app.slug);
        forget(state, &app.id).await?;
        return Err(e);
    }
    for_app(state, &app.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the instance vanished"))
}

fn provision(
    platform: &dyn Platform,
    slug: &str,
    port: u16,
    password: &str,
    maxmemory_mb: u32,
) -> anyhow::Result<()> {
    let dir = dir(slug);
    platform.make_dirs(&dir, 0o750)?;
    platform.write_file(
        &conf_path(slug),
        &render_conf(slug, port, password, maxmemory_mb),
        0o600,
    )?;
    platform.chown_tree(&dir, &user_name(slug))?;
    platform.write_file(&unit_path(slug), &render_unit(slug), 0o644)?;
    platform.service(ServiceAction::DaemonReload, "")?;
    platform.service(ServiceAction::EnableNow, &unit_name(slug))?;
    Ok(())
}

fn remove_from_host(platform: &dyn Platform, slug: &str) {
    let unit = unit_name(slug);
    let _ = platform.service(ServiceAction::Stop, &unit);
    let _ = platform.service(ServiceAction::Disable, &unit);
    let _ = platform.remove_file(&unit_path(slug));
    let _ = platform.service(ServiceAction::DaemonReload, "");
    let _ = platform.remove_tree(&dir(slug));
}

async fn forget(state: &State, app_id: &str) -> anyhow::Result<()> {
    let mut tx = state.pool.begin().await?;
    sqlx::query!("DELETE FROM redis_instances WHERE app_id = ?", app_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query!(
        "DELETE FROM app_ports WHERE app_id = ? AND name = ?",
        app_id,
        PORT_NAME
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn release(state: &State, platform: &dyn Platform, app: &App) -> anyhow::Result<bool> {
    if for_app(state, &app.id).await?.is_none() {
        return Ok(false);
    }
    let unit = unit_name(&app.slug);
    let _ = platform.service(ServiceAction::Stop, &unit);
    let _ = platform.service(ServiceAction::Disable, &unit);
    platform.remove_file(&unit_path(&app.slug))?;
    platform.service(ServiceAction::DaemonReload, "")?;
    platform.remove_tree(&dir(&app.slug))?;
    forget(state, &app.id).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::apps::{self, provision};
    use ferrum_platform::FakePlatform;

    fn position(calls: &[String], needle: &str) -> usize {
        calls
            .iter()
            .position(|c| c == needle)
            .unwrap_or_else(|| panic!("no call {needle:?} in {calls:#?}"))
    }

    #[test]
    fn the_config_is_loopback_password_protected_and_never_evicts() {
        let conf = render_conf("ledger", 20001, "pw", 64);
        for line in [
            "bind 127.0.0.1",
            "port 20001",
            "requirepass pw",
            "maxmemory 64mb",
            "maxmemory-policy noeviction",
            "appendonly yes",
            "appendonlydir appendonlydir",
            "dir /var/lib/ferrum/redis/ledger",
            "supervised systemd",
            "daemonize no",
            "protected-mode yes",
            "save \"\"",
            "logfile \"\"",
        ] {
            assert!(
                conf.contains(&format!("{line}\n")),
                "missing {line}\n{conf}"
            );
        }
    }

    #[test]
    fn the_unit_runs_as_the_app_user_and_is_confined_to_its_directory() {
        let u = render_unit("ledger");
        for line in [
            "User=ferrum-ledger",
            "Group=ferrum-ledger",
            "ExecStart=/usr/bin/redis-server /var/lib/ferrum/redis/ledger/redis.conf",
            "Type=notify",
            "Restart=always",
            "ProtectSystem=strict",
            "ReadWritePaths=/var/lib/ferrum/redis/ledger",
            "NoNewPrivileges=yes",
            "WantedBy=multi-user.target",
        ] {
            assert!(u.contains(&format!("{line}\n")), "missing {line}\n{u}");
        }
        assert_eq!(
            unit_path("ledger"),
            Path::new("/etc/systemd/system/ferrum-redis-ledger.service")
        );
        assert_eq!(url(20001, "pw"), "redis://:pw@127.0.0.1:20001/0");
    }

    #[tokio::test]
    async fn requesting_redis_reserves_a_port_writes_the_files_and_starts_the_unit() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let instance = request(&state, &p, &app, 64).await.unwrap();
        assert!(ports::RANGE.contains(&instance.port));
        assert_ne!(instance.port, app.routes[0].port);
        assert!(instance.created_at.ends_with('Z'));
        let calls = p.calls();
        let conf = position(
            &calls,
            "write_file /var/lib/ferrum/redis/ledger/redis.conf 600",
        );
        let chown = position(
            &calls,
            "chown_tree /var/lib/ferrum/redis/ledger ferrum-ledger",
        );
        let unit = position(
            &calls,
            "write_file /etc/systemd/system/ferrum-redis-ledger.service 644",
        );
        let reload = position(&calls, "service daemon-reload ");
        let start = position(&calls, "service enable-now ferrum-redis-ledger");
        assert!(
            conf < chown && chown < unit && unit < reload && reload < start,
            "{calls:#?}"
        );
        let written = p
            .written("/var/lib/ferrum/redis/ledger/redis.conf")
            .unwrap();
        assert!(written.contains(&format!("port {}\n", instance.port)));
        let e = request(&state, &p, &app, 64).await.unwrap_err();
        assert!(
            matches!(e.downcast_ref::<RedisError>(), Some(RedisError::Exists(_))),
            "one instance per app"
        );
        assert_eq!(
            for_app(&state, &app.id).await.unwrap().unwrap().port,
            instance.port
        );
        assert_eq!(list(&state).await.unwrap()[0].app_slug, "ledger");
        let url = url_for(&state, &app.id).await.unwrap().unwrap();
        assert!(
            url.starts_with("redis://:")
                && url.ends_with(&format!("@127.0.0.1:{}/0", instance.port))
        );
    }

    #[tokio::test]
    async fn a_host_that_refuses_leaves_no_instance_and_no_port_behind() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        p.fail_next("service enable-now");
        assert!(request(&state, &p, &app, 64).await.is_err());
        assert!(for_app(&state, &app.id).await.unwrap().is_none());
        let ports: i64 = sqlx::query_scalar("SELECT count(*) FROM app_ports WHERE name = 'redis'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(ports, 0);
        assert!(p.removed("/etc/systemd/system/ferrum-redis-ledger.service"));
        assert!(
            request(&state, &p, &app, 8).await.is_err(),
            "too little memory"
        );
    }

    #[tokio::test]
    async fn releasing_redis_stops_removes_and_frees_the_port() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        request(&state, &p, &app, 64).await.unwrap();
        assert!(release(&state, &p, &app).await.unwrap());
        let calls = p.calls();
        let stop = position(&calls, "service stop ferrum-redis-ledger");
        let tree = position(&calls, "remove_tree /var/lib/ferrum/redis/ledger");
        assert!(stop < tree);
        assert!(p.removed("/etc/systemd/system/ferrum-redis-ledger.service"));
        let ports: i64 = sqlx::query_scalar("SELECT count(*) FROM app_ports WHERE name = 'redis'")
            .fetch_one(&state.pool)
            .await
            .unwrap();
        assert_eq!(ports, 0);
        assert!(!release(&state, &p, &app).await.unwrap());
        assert!(list(&state).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn deprovisioning_an_app_takes_its_redis_with_it() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        request(&state, &p, &app, 64).await.unwrap();
        provision::deprovision(&state, &p, &app).await.unwrap();
        let calls = p.calls();
        let redis = position(&calls, "service stop ferrum-redis-ledger");
        let user = position(&calls, "remove_system_user ferrum-ledger");
        assert!(redis < user, "{calls:#?}");
        assert!(for_app(&state, &app.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn an_app_update_keeps_the_redis_port_and_a_route_cannot_claim_its_name() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let instance = request(&state, &p, &app, 64).await.unwrap();
        apps::update(
            &state,
            "ledger",
            apps::AppChanges {
                memory_mb: Some(1024),
                ..apps::AppChanges::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            for_app(&state, &app.id).await.unwrap().unwrap().port,
            instance.port
        );
        let e = apps::update(
            &state,
            "ledger",
            apps::AppChanges {
                routes: Some(vec![apps::NewRoute {
                    path: "/".into(),
                    port_name: "redis".into(),
                    websocket: false,
                }]),
                ..apps::AppChanges::default()
            },
        )
        .await
        .unwrap_err();
        assert!(e.to_string().contains("redis"), "{e}");
    }
}
