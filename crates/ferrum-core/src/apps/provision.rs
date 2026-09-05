use super::unit::{render_unit, unit_name, unit_path};
use super::vhost::{custom_path, render_vhost, vhost_path};
use super::{App, env};
use crate::deploy::maintenance;
use crate::runtime::toolchain::Store;
use crate::state::State;
use crate::{APPS_DIR, acme, nginx, redis};
use anyhow::Context;
use ferrum_platform::ubuntu::NGINX_UNIT;
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
        platform.make_dirs(&dir.join(sub), mode)?;
    }
    platform.chown_tree(&dir, &user)?;
    write_env(state, platform, app).await?;

    if app.runtime.has_process() {
        let toolchain = Store::default().dir(app.toolchain, &app.runtime_version);
        let unit = render_unit(app, &toolchain)?;
        platform.write_file(&unit_path(&app.slug), &unit, 0o644)?;
    } else {
        platform.remove_file(&unit_path(&app.slug))?;
    }
    platform.service(ServiceAction::DaemonReload, "")?;

    let custom = custom_path(&app.slug);
    if !platform.file_exists(&custom) {
        platform.write_file(&custom, "", 0o644)?;
    }
    maintenance::ensure_page(platform)?;
    let with_tls: Vec<String> = app
        .domains
        .iter()
        .filter(|d| platform.file_exists(&acme::cert_dir(d).join("fullchain.pem")))
        .cloned()
        .collect();
    let vhost = render_vhost(app, &app.domains, &with_tls);
    nginx::replace_and_reload(platform, &vhost_path(&app.slug), &vhost)
        .context("nginx refused the generated site configuration")?;
    Ok(())
}

pub async fn write_env(state: &State, platform: &dyn Platform, app: &App) -> anyhow::Result<()> {
    let vars = env::all(state, &app.id).await?;
    let managed = env::managed_for(state, app).await?;
    let env_path = app_dir(&app.slug).join("shared/.env");
    platform.write_file(&env_path, &env::render(&vars, &managed, &app.routes), 0o600)?;
    platform.chown_tree(&env_path, &user_name(&app.slug))?;
    Ok(())
}

pub async fn reprovision(state: &State, platform: &dyn Platform, app: &App) -> anyhow::Result<()> {
    provision(state, platform, app).await
}

pub async fn deprovision(state: &State, platform: &dyn Platform, app: &App) -> anyhow::Result<()> {
    redis::release(state, platform, app).await?;
    let unit = unit_name(&app.slug);
    let _ = platform.service(ServiceAction::Stop, &unit);
    let _ = platform.service(ServiceAction::Disable, &unit);
    platform.remove_file(&unit_path(&app.slug))?;
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
            "write_file /etc/systemd/system/ferrum-app-ledger.service 644",
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
        assert!(
            calls.contains(&"chown_tree /var/lib/ferrum/apps/ledger ferrum-ledger".to_string())
        );
        assert!(calls.contains(
            &"chown_tree /var/lib/ferrum/apps/ledger/shared/.env ferrum-ledger".to_string()
        ));
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
    async fn the_env_file_is_owned_by_the_app_user_and_contains_the_ports() {
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
        assert!(contents.contains(&format!("PORT={}\n", app.routes[0].port)));
        assert!(contents.contains(&format!("WS_PORT={}\n", app.routes[1].port)));
        assert!(contents.contains("HOST=127.0.0.1\n"));
        assert!(contents.starts_with("SECRET=hunter2\n"));
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
        assert!(contents.contains("@127.0.0.1:5432/ledger_prod\nPORT="));
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
    async fn a_static_app_gets_no_unit_and_a_leftover_one_is_removed() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let mut new = new_app("docs", &[("/", "main", false)]);
        new.runtime = crate::runtime::RuntimeKind::Static;
        new.output_dir = Some("dist".into());
        let app = create(&state, new).await.unwrap();
        provision(&state, &platform, &app).await.unwrap();
        assert!(
            platform
                .written("/etc/systemd/system/ferrum-app-docs.service")
                .is_none()
        );
        assert!(
            platform
                .calls()
                .contains(&"remove_file /etc/systemd/system/ferrum-app-docs.service".to_string())
        );
        assert!(
            platform
                .written("/etc/nginx/conf.d/ferrum-docs.conf")
                .unwrap()
                .contains("root /var/lib/ferrum/apps/docs/current/dist;")
        );
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
    async fn deprovisioning_removes_the_unit_the_vhost_the_user_and_the_directory() {
        let (_d, state) = state().await;
        let platform = FakePlatform::new();
        let app = create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        provision(&state, &platform, &app).await.unwrap();
        deprovision(&state, &platform, &app).await.unwrap();

        let calls = platform.calls();
        assert!(calls.contains(&"service stop ferrum-app-ledger".to_string()));
        assert!(platform.removed("/etc/systemd/system/ferrum-app-ledger.service"));
        assert!(platform.removed("/etc/nginx/conf.d/ferrum-ledger.conf"));
        assert!(calls.contains(&"remove_system_user ferrum-ledger".to_string()));
        assert!(calls.contains(&"remove_tree /var/lib/ferrum/apps/ledger".to_string()));
        let stop = position(&calls, "service stop ferrum-app-ledger");
        let user = position(&calls, "remove_system_user ferrum-ledger");
        let tree = position(&calls, "remove_tree /var/lib/ferrum/apps/ledger");
        assert!(stop < user && user < tree, "{calls:#?}");
    }
}
