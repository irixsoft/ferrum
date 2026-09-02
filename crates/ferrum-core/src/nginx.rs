use crate::{ACME_ROOT, LISTEN_ADDR};
use ferrum_platform::ubuntu::{NGINX_CONF_DIR, NGINX_UNIT};
use ferrum_platform::{Platform, PlatformError, ServiceAction};
use std::path::{Path, PathBuf};

const ACME_CONF: &str = include_str!("../../../packaging/nginx-acme.conf");
const PANEL_TMPL: &str = include_str!("../../../packaging/nginx-panel.conf.tmpl");

const REPO_KEY_URL: &str = "https://nginx.org/keys/nginx_signing.key";

pub fn acme_conf_path() -> PathBuf {
    Path::new(NGINX_CONF_DIR).join("ferrum-acme.conf")
}

pub fn panel_conf_path() -> PathBuf {
    Path::new(NGINX_CONF_DIR).join("ferrum-panel.conf")
}

pub fn install(platform: &dyn Platform, codename: &str) -> Result<(), PlatformError> {
    platform.add_apt_repo(
        "nginx",
        REPO_KEY_URL,
        &format!("https://nginx.org/packages/ubuntu {codename} nginx"),
    )?;
    platform.install_packages(&["nginx"])?;
    platform.write_file(&acme_conf_path(), ACME_CONF, 0o644)?;
    platform.nginx_test()?;
    platform.service(ServiceAction::EnableNow, NGINX_UNIT)
}

pub fn render_panel_vhost(hostname: &str, cert_dir: &Path) -> String {
    PANEL_TMPL
        .replace("{{hostname}}", hostname)
        .replace("{{cert_dir}}", &cert_dir.to_string_lossy())
        .replace("{{acme_root}}", ACME_ROOT)
        .replace("{{upstream}}", &LISTEN_ADDR.to_string())
}

pub fn write_and_reload(
    platform: &dyn Platform,
    path: &Path,
    contents: &str,
) -> Result<(), PlatformError> {
    platform.write_file(path, contents, 0o644)?;
    platform.nginx_test()?;
    platform.service(ServiceAction::Reload, NGINX_UNIT)
}

/// Like `write_and_reload`, but a config that fails `nginx -t` is rolled back to whatever was
/// there before so one bad site never takes the others down.
pub fn replace_and_reload(
    platform: &dyn Platform,
    path: &Path,
    contents: &str,
) -> Result<(), PlatformError> {
    let previous = platform.read_file(path)?;
    platform.write_file(path, contents, 0o644)?;
    if let Err(failed) = platform.nginx_test() {
        match previous {
            Some(old) => platform.write_file(path, &old, 0o644)?,
            None => platform.remove_file(path)?,
        }
        return Err(failed);
    }
    platform.service(ServiceAction::Reload, NGINX_UNIT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ACME_WEBROOT;
    use ferrum_platform::FakePlatform;
    use std::path::Path;

    #[test]
    fn install_adds_upstream_repo_before_installing() {
        let p = FakePlatform::new();
        install(&p, "noble").unwrap();
        let calls = p.calls();
        let repo = calls
            .iter()
            .position(|c| c.starts_with("add_apt_repo nginx"))
            .unwrap();
        let inst = calls
            .iter()
            .position(|c| c.starts_with("install_packages nginx"))
            .unwrap();
        assert!(repo < inst, "repo must be added before install: {calls:?}");
    }

    #[test]
    fn install_serves_challenges_before_nginx_starts() {
        let p = FakePlatform::new();
        install(&p, "noble").unwrap();
        let calls = p.calls();
        let conf = calls
            .iter()
            .position(|c| c.contains("ferrum-acme.conf"))
            .unwrap();
        let start = calls
            .iter()
            .position(|c| c == "service enable-now nginx")
            .unwrap();
        assert!(conf < start, "{calls:?}");
    }

    #[test]
    fn panel_vhost_proxies_to_loopback_and_serves_acme() {
        let conf = render_panel_vhost(
            "panel.example.com",
            Path::new("/var/lib/ferrum/certs/panel.example.com"),
        );
        assert!(conf.contains("server_name panel.example.com;"));
        assert!(conf.contains("proxy_pass http://127.0.0.1:8443;"));
        assert!(conf.contains("listen 443 ssl;"));
        assert!(conf.contains("http2 on;"));
        assert!(conf.contains("/var/lib/ferrum/certs/panel.example.com/fullchain.pem"));
        assert!(conf.contains("return 301 https://$host$request_uri;"));
    }

    #[test]
    fn panel_vhost_carries_websocket_and_a_long_read_timeout() {
        let conf = render_panel_vhost("p.example.com", Path::new("/c"));
        assert!(conf.contains("proxy_set_header Upgrade $http_upgrade;"));
        assert!(conf.contains("proxy_set_header Connection $connection_upgrade;"));
        assert!(conf.contains("proxy_read_timeout 3600s;"));
    }

    #[test]
    fn panel_vhost_leaves_no_placeholder_behind() {
        let conf = render_panel_vhost("p.example.com", Path::new("/c"));
        assert!(!conf.contains("{{"), "{conf}");
    }

    #[test]
    fn the_upgrade_map_is_defined_exactly_once() {
        let panel = render_panel_vhost("p.example.com", Path::new("/c"));
        assert!(ACME_CONF.contains("map $http_upgrade $connection_upgrade"));
        assert!(!panel.contains("map $http_upgrade"));
    }

    #[test]
    fn the_challenge_root_matches_the_shipped_config() {
        assert!(ACME_CONF.contains(&format!("root {ACME_ROOT};")));
        assert!(ACME_WEBROOT.starts_with(ACME_ROOT));
    }

    #[test]
    fn write_and_reload_validates_before_reloading() {
        let p = FakePlatform::new();
        write_and_reload(&p, Path::new("/etc/nginx/conf.d/x.conf"), "server {}").unwrap();
        let calls = p.calls();
        let w = calls
            .iter()
            .position(|c| c.starts_with("write_file"))
            .unwrap();
        let t = calls.iter().position(|c| c == "nginx_test").unwrap();
        let r = calls
            .iter()
            .position(|c| c == "service reload nginx")
            .unwrap();
        assert!(w < t && t < r, "{calls:?}");
    }

    #[test]
    fn a_replacement_that_fails_the_test_restores_the_previous_config() {
        let p = FakePlatform::new();
        let path = Path::new("/etc/nginx/conf.d/x.conf");
        replace_and_reload(&p, path, "good").unwrap();
        p.fail_next("nginx_test");
        assert!(replace_and_reload(&p, path, "bad").is_err());
        assert_eq!(
            p.written("/etc/nginx/conf.d/x.conf").as_deref(),
            Some("good")
        );
        assert_eq!(p.calls_matching("service reload nginx").len(), 1);
    }

    #[test]
    fn a_first_config_that_fails_the_test_is_removed() {
        let p = FakePlatform::new();
        let path = Path::new("/etc/nginx/conf.d/x.conf");
        p.fail_next("nginx_test");
        assert!(replace_and_reload(&p, path, "bad").is_err());
        assert!(p.removed("/etc/nginx/conf.d/x.conf"));
    }

    #[test]
    fn a_failing_config_test_prevents_the_reload() {
        let p = FakePlatform::new();
        p.fail_next("nginx_test");
        assert!(write_and_reload(&p, Path::new("/etc/nginx/conf.d/x.conf"), "bad").is_err());
        assert!(!p.calls().iter().any(|c| c == "service reload nginx"));
    }
}
