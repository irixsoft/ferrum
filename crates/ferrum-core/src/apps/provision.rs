use super::processes::{legacy_unit_name, legacy_unit_path, unit_path, unit_prefix, valid_name};
use super::unit::render_unit;
use super::vhost::{custom_path, render_vhost, vhost_path};
use super::{App, env};
use crate::deploy::maintenance;
use crate::runtime::toolchain::{self, Store};
use crate::state::State;
use crate::{APPS_DIR, acme, apps, nginx, redis};
use anyhow::Context;
use ferrum_platform::ubuntu::{NGINX_UNIT, SYSTEMD_UNIT_DIR};
use ferrum_platform::{Platform, ServiceAction};
use std::path::{Path, PathBuf};

pub fn app_dir(slug: &str) -> PathBuf {
    Path::new(APPS_DIR).join(slug)
}

pub fn user_name(slug: &str) -> String {
    format!("ferrum-{slug}")
}

pub async fn provision(state: &State, platform: &dyn Platform, app: &App) -> anyhow::Result<()> {
    let dir = app_dir(&app.slug);
    let user = user_name(&app.slug);
    if !platform.user_exists(&user) {
        platform
            .create_system_user(&user, &dir)
            .with_context(|| format!("creating the system user {user}"))?;
    }

    for (sub, mode) in [
        ("", 0o755),
        ("releases", 0o755),
        ("shared", 0o750),
        ("shared/cache", 0o750),
        ("shared/storage", 0o750),
    ] {
        let path = if sub.is_empty() {
            dir.clone()
        } else {
            dir.join(sub)
        };
        platform.make_dirs(&path, mode)?;
        platform.chown(&path, &user)?;
    }
    write_env(state, platform, app).await?;
    write_units(state, platform, app).await?;

    let custom = custom_path(&app.slug);
    if !platform.file_exists(&custom) {
        platform.write_file(&custom, "", 0o644)?;
    }
    maintenance::ensure_page(platform)?;
    nginx::replace_and_reload(platform, &vhost_path(&app.slug), &render_for(platform, app))
        .context("nginx refused the generated site configuration")?;
    Ok(())
}

/// One unit per command process; a unit for a process the app no longer has is stopped and
/// removed, the pre-process unit `ferrum-app-<slug>` included. Ends with a daemon reload.
pub async fn write_units(state: &State, platform: &dyn Platform, app: &App) -> anyhow::Result<()> {
    let store = Store::default();
    let extra = toolchain::extra_for(state, &store, app).await?;
    let toolchain_dir = store.dir(app.toolchain, &app.runtime_version);
    let mut wanted = Vec::new();
    for process in app.command_processes() {
        let unit = render_unit(app, process, &toolchain_dir, extra.as_deref())?;
        let path = unit_path(&app.slug, &process.name);
        platform.write_file(&path, &unit, 0o644)?;
        wanted.push(path);
    }
    let stale = stale_units(platform, app, &wanted)?;
    for path in &stale {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = platform.service(ServiceAction::Stop, &name);
        let _ = platform.service(ServiceAction::Disable, &name);
        platform.remove_file(path)?;
    }
    if !wanted.is_empty() || !stale.is_empty() {
        platform.service(ServiceAction::DaemonReload, "")?;
    }
    Ok(())
}

fn stale_units(
    platform: &dyn Platform,
    app: &App,
    wanted: &[PathBuf],
) -> anyhow::Result<Vec<PathBuf>> {
    let prefix = unit_prefix(&app.slug);
    let mut stale: Vec<PathBuf> = platform
        .list_dir(Path::new(SYSTEMD_UNIT_DIR))
        .unwrap_or_default()
        .into_iter()
        .filter(|name| {
            name.strip_prefix(&prefix)
                .and_then(|rest| rest.strip_suffix(".service"))
                .is_some_and(valid_name)
        })
        .map(|name| Path::new(SYSTEMD_UNIT_DIR).join(name))
        .filter(|path| !wanted.contains(path))
        .collect();
    let legacy = legacy_unit_path(&app.slug);
    if platform.file_exists(&legacy) {
        stale.push(legacy);
    }
    Ok(stale)
}

/// Brings every app from the one-unit layout to a unit per process, once, at daemon start.
/// A unit that was running comes back as the app's processes.
pub async fn migrate_units(state: &State, platform: &dyn Platform) -> anyhow::Result<usize> {
    let mut migrated = 0;
    for app in apps::list(state).await? {
        let legacy = legacy_unit_path(&app.slug);
        if !platform.file_exists(&legacy) {
            continue;
        }
        let was_active = platform.service_is_active(&legacy_unit_name(&app.slug));
        write_units(state, platform, &app).await?;
        if was_active {
            for process in app.command_processes() {
                platform.service(ServiceAction::EnableNow, &process.unit_name(&app.slug))?;
            }
        }
        migrated += 1;
    }
    Ok(migrated)
}

/// Rewrites a site whose file no longer matches what its certificates call for. A site that is
/// not on disk belongs to an app not yet provisioned or being removed, and is left alone.
pub fn refresh_vhost(platform: &dyn Platform, app: &App) -> anyhow::Result<bool> {
    let path = vhost_path(&app.slug);
    let Some(current) = platform.read_file(&path)? else {
        return Ok(false);
    };
    let vhost = render_for(platform, app);
    if current == vhost {
        return Ok(false);
    }
    nginx::replace_and_reload(platform, &path, &vhost)
        .context("nginx refused the generated site configuration")?;
    Ok(true)
}

fn render_for(platform: &dyn Platform, app: &App) -> String {
    let with_tls: Vec<String> = app
        .domains
        .iter()
        .filter(|d| platform.file_exists(&acme::cert_dir(&d.domain).join("fullchain.pem")))
        .map(|d| d.domain.clone())
        .collect();
    render_vhost(app, &with_tls)
}

pub async fn write_env(state: &State, platform: &dyn Platform, app: &App) -> anyhow::Result<()> {
    let vars = env::all(state, &app.id).await?;
    let managed = env::managed_for(state, app).await?;
    let env_path = app_dir(&app.slug).join("shared/.env");
    platform.write_file(
        &env_path,
        &env::render(&vars, &managed, &app.ports()),
        0o600,
    )?;
    platform.chown(&env_path, &user_name(&app.slug))?;
    Ok(())
}

pub async fn reprovision(state: &State, platform: &dyn Platform, app: &App) -> anyhow::Result<()> {
    provision(state, platform, app).await
}

pub async fn deprovision(state: &State, platform: &dyn Platform, app: &App) -> anyhow::Result<()> {
    redis::release(state, platform, app).await?;
    for unit in stale_units(platform, app, &[])? {
        let name = unit
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = platform.service(ServiceAction::Stop, &name);
        let _ = platform.service(ServiceAction::Disable, &name);
        platform.remove_file(&unit)?;
    }
    platform.service(ServiceAction::DaemonReload, "")?;

    platform.remove_file(&vhost_path(&app.slug))?;
    platform.remove_file(&custom_path(&app.slug))?;
    platform.nginx_test()?;
    platform.service(ServiceAction::Reload, NGINX_UNIT)?;

    platform.remove_system_user(&user_name(&app.slug))?;
    platform.remove_tree(&app_dir(&app.slug))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::processes::NewProcess;
    use crate::apps::tests::{new_app, state};
    use crate::apps::{by_slug, create};
    use ferrum_platform::FakePlatform;

    fn position(calls: &[String], needle: &str) -> usize {
        calls
            .iter()
            .position(|c| c == needle)
            .unwrap_or_else(|| panic!("no call {needle:?} in {calls:#?}"))
    }

    #[tokio::test]
    async fn provisioning_creates_the_user_the_layout_the_env_the_unit_and_the_vhost_in_that_order()
    {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        env::set(&state, &app.id, "SECRET", "x").await.unwrap();

        provision(&state, &platform, &app).await.unwrap();

        let calls = platform.calls();
        let user = position(
            &calls,
            "create_system_user ferrum-ledger /var/lib/ferrum/apps/ledger",
        );
        let env = position(
            &calls,
            "write_file /var/lib/ferrum/apps/ledger/shared/.env 600",
        );
        let unit = position(
            &calls,
            "write_file /etc/systemd/system/ferrum-app-ledger-web.service 644",
        );
        let reload = position(&calls, "service daemon-reload ");
        let vhost = position(
            &calls,
            "write_file /etc/nginx/conf.d/ferrum-ledger.conf 644",
        );
        let test = position(&calls, "nginx_test");
        let nginx = position(&calls, "service reload nginx");
        assert!(
            user < env
                && env < unit
                && unit < reload
                && reload < vhost
                && vhost < test
                && test < nginx,
            "{calls:#?}"
        );
        for path in [
            "",
            "/releases",
            "/shared",
            "/shared/cache",
            "/shared/storage",
            "/shared/.env",
        ] {
            let chown = format!("chown /var/lib/ferrum/apps/ledger{path} ferrum-ledger");
            assert!(calls.contains(&chown), "{calls:#?}");
        }
        assert!(
            !calls.iter().any(|c| c.starts_with("chown_tree")),
            "a recursive chown walks a cache a build may be deleting under it"
        );
        assert!(
            calls.contains(&"make_dirs /var/lib/ferrum/apps/ledger/shared/storage 750".to_string())
        );
        assert!(
            !calls
                .iter()
                .any(|c| c.starts_with("service start") || c.starts_with("service enable")),
            "nothing starts until there is a release"
        );
        assert_eq!(
            platform
                .written("/etc/nginx/ferrum-custom/ledger.conf")
                .as_deref(),
            Some(""),
            "the include target must exist or nginx refuses to start"
        );
    }

    #[tokio::test]
    async fn the_env_file_is_owned_by_the_app_user_and_names_every_port_process() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let app = create(
            &state,
            new_app("ledger", &[("/", "main", false), ("/ws", "ws", true)]),
        )
        .await
        .unwrap();
        env::set(&state, &app.id, "SECRET", "hunter2")
            .await
            .unwrap();
        provision(&state, &platform, &app).await.unwrap();

        let contents = platform
            .written("/var/lib/ferrum/apps/ledger/shared/.env")
            .unwrap();
        assert!(contents.contains(&format!("WEB_PORT={}\n", app.port_of("web").unwrap())));
        assert!(contents.contains(&format!("WS_PORT={}\n", app.port_of("ws").unwrap())));
        assert!(contents.contains("HOST=127.0.0.1\n"));
        assert!(!contents.contains("\nPORT="), "PORT belongs to each unit");
        assert!(contents.starts_with("SECRET=hunter2\n"));
        let web = platform
            .written("/etc/systemd/system/ferrum-app-ledger-web.service")
            .unwrap();
        assert!(web.contains(&format!(
            "Environment=PORT={}\n",
            app.port_of("web").unwrap()
        )));
        let ws = platform
            .written("/etc/systemd/system/ferrum-app-ledger-ws.service")
            .unwrap();
        assert!(ws.contains(&format!(
            "Environment=PORT={}\n",
            app.port_of("ws").unwrap()
        )));
    }

    #[tokio::test]
    async fn a_process_the_app_no_longer_has_loses_its_unit() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let app = create(
            &state,
            new_app("ledger", &[("/", "main", false), ("/ws", "ws", true)]),
        )
        .await
        .unwrap();
        provision(&state, &platform, &app).await.unwrap();
        let only_web = crate::apps::update(
            &state,
            "ledger",
            crate::apps::AppChanges {
                processes: Some(vec![NewProcess::web("bun run start", None)]),
                routes: Some(vec![crate::apps::NewRoute::main()]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        provision(&state, &platform, &only_web).await.unwrap();
        let calls = platform.calls();
        assert!(calls.contains(&"service stop ferrum-app-ledger-ws".to_string()));
        assert!(calls.contains(&"service disable ferrum-app-ledger-ws".to_string()));
        assert!(platform.removed("/etc/systemd/system/ferrum-app-ledger-ws.service"));
        assert!(
            platform
                .written("/etc/systemd/system/ferrum-app-ledger-web.service")
                .is_some()
        );
    }

    #[tokio::test]
    async fn another_app_whose_slug_starts_the_same_keeps_its_units() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let other = "/etc/systemd/system/ferrum-app-ledger-2-web.service";
        platform
            .write_file(Path::new(other), "[Unit]\n", 0o644)
            .unwrap();
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let before = platform.calls().len();
        provision(&state, &platform, &app).await.unwrap();
        deprovision(&state, &platform, &app).await.unwrap();
        assert!(!platform.removed(other));
        let calls: Vec<String> = platform.calls().into_iter().skip(before).collect();
        assert!(
            !calls.iter().any(|c| c.contains("ferrum-app-ledger-2-web")),
            "{calls:#?}"
        );
    }

    #[tokio::test]
    async fn a_linked_database_reaches_the_env_file_on_the_next_write() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        crate::postgres::create(
            &state,
            &platform,
            crate::postgres::tests::new("ledger_prod"),
        )
        .await
        .unwrap();
        crate::postgres::link(&state, &app.id, "ledger_prod")
            .await
            .unwrap();
        write_env(&state, &platform, &app).await.unwrap();
        let contents = platform
            .written("/var/lib/ferrum/apps/ledger/shared/.env")
            .unwrap();
        assert!(
            contents.starts_with("DATABASE_URL=postgres://ledger_prod:"),
            "{contents}"
        );
        assert!(contents.contains("@127.0.0.1:5432/ledger_prod\nWEB_PORT="));
    }

    #[tokio::test]
    async fn a_failing_nginx_test_rolls_the_vhost_back_and_keeps_the_app() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        platform.fail_next("nginx_test");
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();

        let e = provision(&state, &platform, &app).await.unwrap_err();
        assert!(e.to_string().contains("nginx refused"), "{e:#}");
        assert!(
            platform.removed("/etc/nginx/conf.d/ferrum-ledger.conf"),
            "a vhost that fails nginx -t must not stay and break every site"
        );
        assert!(by_slug(&state, "ledger").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn a_reprovision_that_fails_keeps_the_previous_vhost() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        provision(&state, &platform, &app).await.unwrap();
        let before = platform
            .written("/etc/nginx/conf.d/ferrum-ledger.conf")
            .unwrap();

        platform.fail_next("nginx_test");
        assert!(reprovision(&state, &platform, &app).await.is_err());
        assert_eq!(
            platform
                .written("/etc/nginx/conf.d/ferrum-ledger.conf")
                .as_deref(),
            Some(before.as_str()),
            "the last good vhost is restored"
        );
    }

    #[tokio::test]
    async fn provisioning_twice_is_harmless_and_does_not_rewrite_the_user_snippet() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        provision(&state, &platform, &app).await.unwrap();
        platform
            .write_file(
                Path::new("/etc/nginx/ferrum-custom/ledger.conf"),
                "# mine",
                0o644,
            )
            .unwrap();
        provision(&state, &platform, &app).await.unwrap();
        assert_eq!(
            platform.calls_matching("create_system_user").len(),
            1,
            "an existing user is not created again"
        );
        assert_eq!(
            platform
                .written("/etc/nginx/ferrum-custom/ledger.conf")
                .as_deref(),
            Some("# mine")
        );
    }

    #[tokio::test]
    async fn a_folder_app_gets_no_unit_and_a_leftover_one_is_removed() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let mut new = new_app("docs", &[("/", "main", false)]);
        new.processes = vec![NewProcess::folder("web", "dist")];
        let app = create(&state, new).await.unwrap();
        platform
            .write_file(
                Path::new("/etc/systemd/system/ferrum-app-docs.service"),
                "[Unit]",
                0o644,
            )
            .unwrap();
        provision(&state, &platform, &app).await.unwrap();
        assert!(
            platform
                .written("/etc/systemd/system/ferrum-app-docs-web.service")
                .is_none()
        );
        assert!(platform.removed("/etc/systemd/system/ferrum-app-docs.service"));
        assert!(
            platform
                .written("/etc/nginx/conf.d/ferrum-docs.conf")
                .unwrap()
                .contains("root /var/lib/ferrum/apps/docs/current/dist;")
        );
    }

    #[tokio::test]
    async fn the_one_unit_layout_is_migrated_once_and_a_running_app_comes_back() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        create(&state, new_app("idle", &[("/", "main", false)]))
            .await
            .unwrap();
        platform
            .write_file(
                Path::new("/etc/systemd/system/ferrum-app-ledger.service"),
                "[Unit]",
                0o644,
            )
            .unwrap();
        platform.set_active("ferrum-app-ledger");
        assert_eq!(migrate_units(&state, &platform).await.unwrap(), 1);
        let calls = platform.calls();
        let stop = position(&calls, "service stop ferrum-app-ledger");
        let write = position(
            &calls,
            "write_file /etc/systemd/system/ferrum-app-ledger-web.service 644",
        );
        let reload = position(&calls, "service daemon-reload ");
        let start = position(&calls, "service enable-now ferrum-app-ledger-web");
        assert!(
            write < stop && stop < reload && reload < start,
            "{calls:#?}"
        );
        assert!(platform.removed("/etc/systemd/system/ferrum-app-ledger.service"));
        assert!(
            platform
                .written("/etc/systemd/system/ferrum-app-idle-web.service")
                .is_none(),
            "an app without the old unit is left alone"
        );
        assert_eq!(migrate_units(&state, &platform).await.unwrap(), 0);
        let _ = app;
    }

    #[tokio::test]
    async fn a_certificate_on_disk_turns_tls_on() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        platform
            .write_file(
                Path::new("/var/lib/ferrum/certs/ledger.example.com/fullchain.pem"),
                "cert",
                0o644,
            )
            .unwrap();
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        provision(&state, &platform, &app).await.unwrap();
        let vhost = platform
            .written("/etc/nginx/conf.d/ferrum-ledger.conf")
            .unwrap();
        assert!(vhost.contains("listen 443 ssl;"));
    }

    #[tokio::test]
    async fn deprovisioning_removes_every_unit_the_vhost_the_user_and_the_directory() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let app = create(
            &state,
            new_app("ledger", &[("/", "main", false), ("/ws", "ws", true)]),
        )
        .await
        .unwrap();
        provision(&state, &platform, &app).await.unwrap();
        deprovision(&state, &platform, &app).await.unwrap();

        let calls = platform.calls();
        assert!(calls.contains(&"service stop ferrum-app-ledger-web".to_string()));
        assert!(calls.contains(&"service stop ferrum-app-ledger-ws".to_string()));
        assert!(platform.removed("/etc/systemd/system/ferrum-app-ledger-web.service"));
        assert!(platform.removed("/etc/systemd/system/ferrum-app-ledger-ws.service"));
        assert!(platform.removed("/etc/nginx/conf.d/ferrum-ledger.conf"));
        assert!(calls.contains(&"remove_system_user ferrum-ledger".to_string()));
        assert!(calls.contains(&"remove_tree /var/lib/ferrum/apps/ledger".to_string()));
        let stop = position(&calls, "service stop ferrum-app-ledger-web");
        let user = position(&calls, "remove_system_user ferrum-ledger");
        let tree = position(&calls, "remove_tree /var/lib/ferrum/apps/ledger");
        assert!(stop < user && user < tree, "{calls:#?}");
    }
}
