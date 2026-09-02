use super::App;
use super::provision::app_dir;
use crate::deploy::maintenance;
use crate::{ACME_ROOT, PAGES_DIR, acme};
use ferrum_platform::ubuntu::{NGINX_CONF_DIR, NGINX_CUSTOM_DIR};
use std::fmt::Write;
use std::path::{Path, PathBuf};

const TLS: &str = "    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_prefer_server_ciphers off;
    ssl_ciphers ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305;
    ssl_session_cache shared:ferrum_tls:10m;
    ssl_session_timeout 1d;
    ssl_session_tickets off;

    add_header Strict-Transport-Security \"max-age=31536000; includeSubDomains\" always;
";

const HEADERS: &str = "    add_header X-Content-Type-Options \"nosniff\" always;
    add_header Referrer-Policy \"strict-origin-when-cross-origin\" always;

    client_max_body_size 64m;
    gzip on;
    gzip_vary on;
    gzip_types text/css text/javascript application/javascript application/json image/svg+xml;
";

const PROXY_TIMEOUT: &str = "3600s";
const WEBSOCKET_TIMEOUT: &str = "86400s";

pub fn vhost_path(slug: &str) -> PathBuf {
    Path::new(NGINX_CONF_DIR).join(format!("ferrum-{slug}.conf"))
}

pub fn custom_path(slug: &str) -> PathBuf {
    Path::new(NGINX_CUSTOM_DIR).join(format!("{slug}.conf"))
}

/// `with_tls` names the domains whose certificate is on disk; each gets its own `:443` block.
pub fn render_vhost(app: &App, domains: &[String], with_tls: &[String]) -> String {
    let mut out = format!(
        "# managed by Ferrum — do not edit. Your own directives go in {}\n\n",
        custom_path(&app.slug).display()
    );
    let Some(primary) = domains.first() else {
        return out;
    };
    let primary_tls = with_tls.contains(primary);

    out.push_str("server {\n    listen 80;\n    listen [::]:80;\n");
    let _ = writeln!(out, "    server_name {primary};\n");
    out.push_str(&acme());
    if primary_tls {
        out.push_str("    location / {\n        return 301 https://$host$request_uri;\n    }\n}\n");
    } else {
        out.push_str(HEADERS);
        out.push_str(&body(app));
        out.push_str("}\n");
    }

    if primary_tls {
        out.push_str(&tls_server(primary));
        out.push_str(HEADERS);
        out.push_str(&acme());
        out.push_str(&body(app));
        out.push_str("}\n");
    }

    let scheme = if primary_tls { "https" } else { "$scheme" };
    for secondary in &domains[1..] {
        out.push_str("\nserver {\n    listen 80;\n    listen [::]:80;\n");
        let _ = writeln!(out, "    server_name {secondary};\n");
        out.push_str(&acme());
        let _ = writeln!(
            out,
            "    location / {{\n        return 301 {scheme}://{primary}$request_uri;\n    }}\n}}"
        );
        if with_tls.contains(secondary) {
            out.push_str(&tls_server(secondary));
            out.push_str(&acme());
            let _ = writeln!(
                out,
                "    location / {{\n        return 301 https://{primary}$request_uri;\n    }}\n}}"
            );
        }
    }
    out
}

fn tls_server(domain: &str) -> String {
    let cert_dir = acme::cert_dir(domain);
    let mut out = String::new();
    out.push_str("\nserver {\n    listen 443 ssl;\n    listen [::]:443 ssl;\n    http2 on;\n");
    let _ = writeln!(out, "    server_name {domain};\n");
    let _ = writeln!(
        out,
        "    ssl_certificate     {0}/fullchain.pem;\n    ssl_certificate_key {0}/key.pem;",
        cert_dir.display()
    );
    out.push_str(TLS);
    out
}

fn acme() -> String {
    format!("    location /.well-known/acme-challenge/ {{\n        root {ACME_ROOT};\n    }}\n")
}

/// The flag file toggles the page with no reload; nginx checks it on every request.
fn maintenance(app: &App) -> String {
    format!(
        "    if (-f {flag}) {{ return 503; }}\n    error_page 503 @maintenance;\n    location @maintenance {{\n        root {pages};\n        add_header Retry-After 10 always;\n        rewrite ^ /{page} break;\n    }}\n\n",
        flag = maintenance::flag_path(&app.slug).display(),
        pages = PAGES_DIR,
        page = maintenance::PAGE_NAME,
    )
}

fn body(app: &App) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "    include {};\n", custom_path(&app.slug).display());
    out.push_str(&maintenance(app));

    if !app.runtime.has_process() {
        let root = app_dir(&app.slug)
            .join("current")
            .join(app.output_dir.as_deref().unwrap_or("dist"));
        let _ = writeln!(out, "    root {};", root.display());
        out.push_str("    index index.html;\n\n");
        out.push_str("    location / {\n        try_files $uri $uri/ /index.html;\n    }\n");
        return out;
    }

    let mut routes: Vec<_> = app.routes.iter().collect();
    routes.sort_by_key(|r| (r.path.len(), r.path.clone()));
    for route in routes {
        let timeout = if route.websocket {
            WEBSOCKET_TIMEOUT
        } else {
            PROXY_TIMEOUT
        };
        let _ = write!(
            out,
            "    location {path} {{
        proxy_pass http://127.0.0.1:{port};
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header X-Forwarded-Host $host;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_read_timeout {timeout};
        proxy_send_timeout {timeout};
    }}
",
            path = route.path,
            port = route.port,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{app, route};
    use crate::runtime::RuntimeKind;

    #[test]
    fn the_vhost_proxies_each_route_to_its_own_port_and_raises_the_websocket_timeout() {
        let mut a = app("ledger");
        a.routes = vec![
            route("/", "main", 20000, false),
            route("/ws", "ws", 20001, true),
        ];
        let v = render_vhost(
            &a,
            &["ledger.example.com".into()],
            &["ledger.example.com".into()],
        );

        let tls = v.find("listen 443 ssl;").unwrap();
        let root = v[tls..].find("location / {").unwrap() + tls;
        let ws = v[tls..].find("location /ws {").unwrap() + tls;
        assert!(
            ws > root,
            "longest-prefix locations must be emitted after /"
        );
        assert!(v[ws..].contains("proxy_pass http://127.0.0.1:20001;"));
        assert!(v[ws..].contains("proxy_read_timeout 86400s;"));
        assert!(v[ws..].contains("proxy_set_header Upgrade $http_upgrade;"));
        assert!(v[root..ws].contains("proxy_pass http://127.0.0.1:20000;"));
        assert!(
            v[root..ws].contains("proxy_set_header Upgrade $http_upgrade;"),
            "same-port upgrades on / must work too"
        );
        assert!(v[root..ws].contains("proxy_read_timeout 3600s;"));
        assert!(v.contains(
            "ssl_certificate     /var/lib/ferrum/certs/ledger.example.com/fullchain.pem;"
        ));
        assert!(v.contains("Strict-Transport-Security"));
        assert!(v.contains("return 301 https://$host$request_uri;"));
    }

    #[test]
    fn the_vhost_includes_the_user_snippet_from_outside_conf_d() {
        let v = render_vhost(&app("ledger"), &["ledger.example.com".into()], &[]);
        assert!(v.contains("include /etc/nginx/ferrum-custom/ledger.conf;"));
        assert!(v.starts_with("# managed by Ferrum"));
        assert!(
            !v.contains("map $http_upgrade"),
            "the map lives once, in the acme conf"
        );
    }

    #[test]
    fn the_vhost_serves_the_maintenance_page_only_while_the_flag_exists() {
        let v = render_vhost(
            &app("ledger"),
            &["ledger.example.com".into()],
            &["ledger.example.com".into()],
        );
        assert_eq!(
            v.matches("if (-f /var/lib/ferrum/apps/ledger/maintenance) { return 503; }")
                .count(),
            1,
            "the :80 block only redirects once TLS is on, so the page is served from :443"
        );
        assert!(v.contains("error_page 503 @maintenance;"));
        assert!(v.contains("add_header Retry-After 10 always;"));
        assert!(v.contains("root /var/lib/ferrum/pages;"));
        assert!(v.contains("rewrite ^ /maintenance.html break;"));
        let plain = render_vhost(&app("ledger"), &["ledger.example.com".into()], &[]);
        assert_eq!(plain.matches("return 503;").count(), 1);
    }

    #[test]
    fn a_secondary_domain_with_its_own_certificate_redirects_over_tls() {
        let v = render_vhost(
            &app("ledger"),
            &["ledger.example.com".into(), "www.ledger.example.com".into()],
            &["ledger.example.com".into(), "www.ledger.example.com".into()],
        );
        assert!(v.contains(
            "ssl_certificate     /var/lib/ferrum/certs/www.ledger.example.com/fullchain.pem;"
        ));
        assert_eq!(v.matches("listen 443 ssl;").count(), 2);
        assert_eq!(
            v.matches("return 301 https://ledger.example.com$request_uri;")
                .count(),
            2
        );
        assert!(!v.contains("$scheme://ledger.example.com"));
    }

    #[test]
    fn without_a_certificate_the_vhost_serves_http_only_and_still_answers_acme() {
        let v = render_vhost(&app("ledger"), &["ledger.example.com".into()], &[]);
        assert!(v.contains("listen 80;"));
        assert!(!v.contains("listen 443"));
        assert!(
            !v.contains("Strict-Transport-Security"),
            "HSTS before TLS exists locks the domain out"
        );
        assert!(v.contains("proxy_pass http://127.0.0.1:20000;"));
        assert!(
            v.contains("location /.well-known/acme-challenge/"),
            "a named server on :80 shadows default_server, so it must answer challenges itself"
        );
    }

    #[test]
    fn a_static_app_serves_current_output_dir_with_a_spa_fallback() {
        let mut a = app("docs");
        a.runtime = RuntimeKind::Static;
        a.output_dir = Some("dist".into());
        let v = render_vhost(&a, &["docs.example.com".into()], &[]);
        assert!(v.contains("root /var/lib/ferrum/apps/docs/current/dist;"));
        assert!(v.contains("try_files $uri $uri/ /index.html;"));
        assert!(!v.contains("proxy_pass"));
    }

    #[test]
    fn secondary_domains_redirect_to_the_primary() {
        let v = render_vhost(
            &app("ledger"),
            &["ledger.example.com".into(), "www.ledger.example.com".into()],
            &[],
        );
        assert!(v.contains("server_name www.ledger.example.com;"));
        assert!(v.contains("return 301 $scheme://ledger.example.com$request_uri;"));
    }

    #[test]
    fn without_a_domain_there_is_nothing_to_serve() {
        let v = render_vhost(&app("ledger"), &[], &[]);
        assert!(!v.contains("server {"));
    }

    #[test]
    fn paths_follow_the_slug() {
        assert_eq!(
            vhost_path("ledger"),
            Path::new("/etc/nginx/conf.d/ferrum-ledger.conf")
        );
        assert_eq!(
            custom_path("ledger"),
            Path::new("/etc/nginx/ferrum-custom/ledger.conf")
        );
    }
}
