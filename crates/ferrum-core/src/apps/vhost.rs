use super::App;
use super::provision::app_dir;
use crate::ACME_ROOT;
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

pub fn render_vhost(app: &App, domains: &[String], cert_dir: Option<&Path>) -> String {
    let mut out = format!(
        "# managed by Ferrum — do not edit. Your own directives go in {}\n\n",
        custom_path(&app.slug).display()
    );
    let Some(primary) = domains.first() else {
        return out;
    };

    out.push_str("server {\n    listen 80;\n    listen [::]:80;\n");
    let _ = writeln!(out, "    server_name {primary};\n");
    out.push_str(&acme());
    match cert_dir {
        Some(_) => {
            out.push_str(
                "    location / {\n        return 301 https://$host$request_uri;\n    }\n}\n",
            );
        }
        None => {
            out.push_str(HEADERS);
            out.push_str(&body(app));
            out.push_str("}\n");
        }
    }

    if let Some(cert_dir) = cert_dir {
        out.push_str("\nserver {\n    listen 443 ssl;\n    listen [::]:443 ssl;\n    http2 on;\n");
        let _ = writeln!(out, "    server_name {primary};\n");
        let _ = writeln!(
            out,
            "    ssl_certificate     {0}/fullchain.pem;\n    ssl_certificate_key {0}/key.pem;",
            cert_dir.display()
        );
        out.push_str(TLS);
        out.push_str(HEADERS);
        out.push_str(&acme());
        out.push_str(&body(app));
        out.push_str("}\n");
    }

    for secondary in &domains[1..] {
        out.push_str("\nserver {\n    listen 80;\n    listen [::]:80;\n");
        let _ = writeln!(out, "    server_name {secondary};\n");
        out.push_str(&acme());
        let _ = writeln!(
            out,
            "    location / {{\n        return 301 $scheme://{primary}$request_uri;\n    }}\n}}"
        );
    }
    out
}

fn acme() -> String {
    format!("    location /.well-known/acme-challenge/ {{\n        root {ACME_ROOT};\n    }}\n")
}

fn body(app: &App) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "    include {};\n", custom_path(&app.slug).display());

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
            Some(Path::new("/var/lib/ferrum/certs/ledger.example.com")),
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
        let v = render_vhost(&app("ledger"), &["ledger.example.com".into()], None);
        assert!(v.contains("include /etc/nginx/ferrum-custom/ledger.conf;"));
        assert!(v.starts_with("# managed by Ferrum"));
        assert!(
            !v.contains("map $http_upgrade"),
            "the map lives once, in the acme conf"
        );
    }

    #[test]
    fn without_a_certificate_the_vhost_serves_http_only_and_still_answers_acme() {
        let v = render_vhost(&app("ledger"), &["ledger.example.com".into()], None);
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
        let v = render_vhost(&a, &["docs.example.com".into()], None);
        assert!(v.contains("root /var/lib/ferrum/apps/docs/current/dist;"));
        assert!(v.contains("try_files $uri $uri/ /index.html;"));
        assert!(!v.contains("proxy_pass"));
    }

    #[test]
    fn secondary_domains_redirect_to_the_primary() {
        let v = render_vhost(
            &app("ledger"),
            &["ledger.example.com".into(), "www.ledger.example.com".into()],
            None,
        );
        assert!(v.contains("server_name www.ledger.example.com;"));
        assert!(v.contains("return 301 $scheme://ledger.example.com$request_uri;"));
    }

    #[test]
    fn without_a_domain_there_is_nothing_to_serve() {
        let v = render_vhost(&app("ledger"), &[], None);
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
