use super::domains::{Domain, Job};
use super::provision::app_dir;
use super::{App, Route};
use crate::deploy::maintenance;
use crate::deploy::steps::work_dir;
use crate::{ACME_ROOT, PAGES_DIR, acme, logs};
use ferrum_platform::ubuntu::{NGINX_CONF_DIR, NGINX_CUSTOM_DIR};
use std::fmt::Write;
use std::path::{Path, PathBuf};

const TLS: &str = "    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_prefer_server_ciphers off;
    ssl_ciphers ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305;
    ssl_session_cache shared:ferrum_tls:10m;
    ssl_session_timeout 1d;
    ssl_session_tickets off;

    add_header Strict-Transport-Security \"max-age=31536000\" always;
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

/// One `:80` block per name, and a `:443` block for each name in `with_tls`, whose certificate
/// is on disk.
pub fn render_vhost(app: &App, with_tls: &[String]) -> String {
    let mut out = format!(
        "# managed by Ferrum — do not edit. Your own directives go in {}\n",
        custom_path(&app.slug).display()
    );
    let tls = |name: &str| with_tls.iter().any(|d| d == name);
    for row in &app.domains {
        let name = row.domain.as_str();
        match row.job {
            Job::Serve => {
                out.push_str(&plain_server(&app.slug, name));
                if tls(name) {
                    out.push_str(
                        "    location / {\n        return 301 https://$host$request_uri;\n    }\n}\n",
                    );
                    out.push_str(&tls_server(&app.slug, name));
                    out.push_str(HEADERS);
                    out.push_str(&acme());
                } else {
                    out.push_str(HEADERS);
                }
                out.push_str(&body(app, &row.target));
                out.push_str("}\n");
            }
            Job::Redirect => {
                let Some(target) = redirect_target(app, row) else {
                    continue;
                };
                out.push_str(&plain_server(&app.slug, name));
                let scheme = if tls(target) { "https" } else { "$scheme" };
                out.push_str(&redirect(scheme, target));
                if tls(name) {
                    out.push_str(&tls_server(&app.slug, name));
                    out.push_str(&acme());
                    let scheme = if tls(target) { "https" } else { "http" };
                    out.push_str(&redirect(scheme, target));
                }
            }
        }
    }
    out
}

/// A redirect lands on a served name; one whose target is gone lands on the primary.
fn redirect_target<'a>(app: &'a App, row: &'a Domain) -> Option<&'a str> {
    let served = |name: &str| app.domain(name).is_some_and(|d| d.serves() && !d.wildcard);
    if served(&row.target) {
        return Some(&row.target);
    }
    app.primary_domain().filter(|p| served(p))
}

fn plain_server(slug: &str, name: &str) -> String {
    let mut out = String::from("\nserver {\n    listen 80;\n    listen [::]:80;\n");
    let _ = writeln!(out, "    server_name {name};");
    out.push_str(&logs(slug));
    out.push_str(&acme());
    out
}

fn redirect(scheme: &str, target: &str) -> String {
    format!("    location / {{\n        return 301 {scheme}://{target}$request_uri;\n    }}\n}}\n")
}

/// Every block Ferrum owns for the app writes to the same pair of files, so the Logs tab reads one
/// access log and one error log per site.
fn logs(slug: &str) -> String {
    format!(
        "    access_log {};\n    error_log {} warn;\n\n",
        logs::access_log_path(slug).display(),
        logs::error_log_path(slug).display()
    )
}

fn tls_server(slug: &str, domain: &str) -> String {
    let cert_dir = acme::cert_dir(domain);
    let mut out = String::new();
    out.push_str("\nserver {\n    listen 443 ssl;\n    listen [::]:443 ssl;\n    http2 on;\n");
    let _ = writeln!(out, "    server_name {domain};");
    out.push_str(&logs(slug));
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

/// `/` belongs to the name's own process; every other path of the app applies on every name.
fn body(app: &App, target: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "    include {};\n", custom_path(&app.slug).display());
    out.push_str(&maintenance(app));

    let root = Route {
        path: "/".into(),
        process: target.into(),
        websocket: app
            .routes
            .iter()
            .any(|r| r.process == target && r.websocket),
    };
    let mut routes: Vec<&Route> = app.routes.iter().filter(|r| r.path != "/").collect();
    routes.push(&root);
    routes.sort_by_key(|r| (r.path.len(), r.path.clone()));
    for route in routes {
        let Some(process) = app.process(&route.process) else {
            continue;
        };
        if let Some(static_dir) = process.static_dir() {
            let root = work_dir(&app_dir(&app.slug).join("current"), &app.root).join(static_dir);
            if route.path == "/" {
                let _ = writeln!(out, "    root {};", root.display());
                out.push_str("    index index.html;\n\n");
                out.push_str(
                    "    location / {\n        try_files $uri $uri/ /index.html;\n    }\n",
                );
            } else {
                let prefix = route.path.trim_end_matches('/');
                let _ = write!(
                    out,
                    "    location {prefix}/ {{\n        alias {root}/;\n        try_files $uri $uri/ {prefix}/index.html;\n    }}\n",
                    root = root.display(),
                );
            }
            continue;
        }
        let Some(port) = process.port else {
            continue;
        };
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
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::domains::NewDomain;
    use crate::apps::tests::{app, folder, process, route, rows};

    fn serve(domain: &str, target: &str, primary: bool) -> NewDomain {
        NewDomain {
            domain: domain.into(),
            job: Job::Serve,
            target: target.into(),
            primary,
            dns_provider_id: None,
        }
    }

    fn redirect_row(domain: &str, target: &str) -> NewDomain {
        NewDomain {
            domain: domain.into(),
            job: Job::Redirect,
            target: target.into(),
            primary: false,
            dns_provider_id: None,
        }
    }

    fn tls(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn ledger_with_www() -> App {
        let mut a = app("ledger");
        a.domains = rows(&[
            serve("ledger.example.com", "web", true),
            redirect_row("www.ledger.example.com", "ledger.example.com"),
        ]);
        a
    }

    fn block<'a>(v: &'a str, listen: &str, name: &str) -> &'a str {
        let marker = format!("{listen}\n    listen [::]:");
        let needle = format!("server_name {name};");
        v.match_indices("\nserver {")
            .map(|(i, _)| {
                let end = v[i + 1..].find("\nserver {").map_or(v.len(), |e| e + i + 1);
                &v[i..end]
            })
            .find(|b| b.contains(&marker) && b.contains(&needle))
            .unwrap_or_else(|| panic!("no {listen} block for {name} in {v}"))
    }

    #[test]
    fn the_vhost_proxies_each_route_to_its_process_port_and_raises_the_websocket_timeout() {
        let mut a = app("ledger");
        a.processes = vec![process("web", 20000), process("ws", 20001)];
        a.routes = vec![route("/", "web", false), route("/ws", "ws", true)];
        let v = render_vhost(&a, &tls(&["ledger.example.com"]));

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
        assert!(v.contains("add_header Strict-Transport-Security \"max-age=31536000\" always;"));
        assert!(
            !v.contains("includeSubDomains"),
            "one app's HSTS must not bind the names of every other app under it"
        );
        assert!(v.contains("return 301 https://$host$request_uri;"));
    }

    #[test]
    fn a_name_serving_a_websocket_process_keeps_the_long_timeout_at_its_root() {
        let mut a = app("ledger");
        a.processes = vec![process("web", 20000), process("ws", 20001)];
        a.routes = vec![route("/", "web", false), route("/ws", "ws", true)];
        a.domains = rows(&[
            serve("ledger.example.com", "web", true),
            serve("ws.ledger.example.com", "ws", false),
        ]);
        let v = render_vhost(&a, &tls(&["ledger.example.com", "ws.ledger.example.com"]));
        let location_root = |b: &str| {
            let start = b.find("location / {").unwrap();
            let end = b[start + 1..]
                .find("location ")
                .map_or(b.len(), |e| e + start + 1);
            b[start..end].to_string()
        };
        let ws = location_root(block(&v, "listen 443 ssl;", "ws.ledger.example.com"));
        assert!(ws.contains("proxy_pass http://127.0.0.1:20001;"), "{ws}");
        assert!(ws.contains("proxy_read_timeout 86400s;"), "{ws}");
        let web = location_root(block(&v, "listen 443 ssl;", "ledger.example.com"));
        assert!(web.contains("proxy_read_timeout 3600s;"), "{web}");
    }

    #[test]
    fn the_vhost_includes_the_user_snippet_from_outside_conf_d() {
        let v = render_vhost(&app("ledger"), &[]);
        assert!(v.contains("include /etc/nginx/ferrum-custom/ledger.conf;"));
        assert!(v.starts_with("# managed by Ferrum"));
        assert!(
            !v.contains("map $http_upgrade"),
            "the map lives once, in the acme conf"
        );
    }

    #[test]
    fn the_vhost_serves_the_maintenance_page_only_while_the_flag_exists() {
        let v = render_vhost(&app("ledger"), &tls(&["ledger.example.com"]));
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
        let plain = render_vhost(&app("ledger"), &[]);
        assert_eq!(plain.matches("return 503;").count(), 1);
    }

    #[test]
    fn a_redirect_row_with_its_own_certificate_redirects_over_tls() {
        let v = render_vhost(
            &ledger_with_www(),
            &tls(&["ledger.example.com", "www.ledger.example.com"]),
        );
        assert!(v.contains(
            "ssl_certificate     /var/lib/ferrum/certs/www.ledger.example.com/fullchain.pem;"
        ));
        assert_eq!(v.matches("listen 443 ssl;").count(), 2);
        let secure = block(&v, "listen 443 ssl;", "www.ledger.example.com");
        assert!(secure.contains("return 301 https://ledger.example.com$request_uri;"));
        assert!(!secure.contains("proxy_pass"));
        let plain = block(&v, "listen 80;", "www.ledger.example.com");
        assert!(plain.contains("return 301 https://ledger.example.com$request_uri;"));
        assert!(plain.contains("location /.well-known/acme-challenge/"));
        assert!(!v.contains("$scheme://ledger.example.com"));
    }

    #[test]
    fn a_redirect_follows_the_scheme_until_its_target_has_a_certificate() {
        let v = render_vhost(&ledger_with_www(), &[]);
        assert!(v.contains("server_name www.ledger.example.com;"));
        assert!(v.contains("return 301 $scheme://ledger.example.com$request_uri;"));
        assert!(!v.contains("listen 443"));
    }

    #[test]
    fn a_redirect_never_points_at_a_redirect() {
        let mut a = app("ledger");
        a.domains = rows(&[
            serve("ledger.example.com", "web", true),
            redirect_row("www.ledger.example.com", "ledger.example.com"),
            redirect_row("old.ledger.example.com", "www.ledger.example.com"),
        ]);
        let v = render_vhost(&a, &[]);
        let old = block(&v, "listen 80;", "old.ledger.example.com");
        assert!(old.contains("return 301 $scheme://ledger.example.com$request_uri;"));
        assert!(!v.contains("://www.ledger.example.com"));
    }

    #[test]
    fn two_served_names_proxy_to_their_own_processes_and_share_the_other_paths() {
        let mut a = app("shop");
        a.processes = vec![
            process("web", 20000),
            process("admin", 20001),
            process("api", 20002),
        ];
        a.routes = vec![route("/", "web", false), route("/api", "api", false)];
        a.domains = rows(&[
            serve("shop.example.com", "web", true),
            serve("admin.shop.example.com", "admin", false),
        ]);
        let v = render_vhost(&a, &[]);
        let shop = block(&v, "listen 80;", "shop.example.com");
        let admin = block(&v, "listen 80;", "admin.shop.example.com");
        let root_of = |b: &str| {
            let at = b.find("location / {").unwrap();
            b[at..].lines().nth(1).unwrap().trim().to_string()
        };
        assert_eq!(root_of(shop), "proxy_pass http://127.0.0.1:20000;");
        assert_eq!(root_of(admin), "proxy_pass http://127.0.0.1:20001;");
        for b in [shop, admin] {
            assert!(b.contains("location /api {"), "{b}");
            assert!(b.contains("proxy_pass http://127.0.0.1:20002;"));
        }
    }

    #[test]
    fn a_wildcard_row_is_its_own_server_name() {
        let mut a = app("shop");
        a.domains = rows(&[
            serve("shop.example.com", "web", true),
            serve("*.shop.example.com", "web", false),
        ]);
        let v = render_vhost(&a, &tls(&["*.shop.example.com"]));
        assert!(v.contains("    server_name *.shop.example.com;\n"));
        assert!(v.contains("    server_name shop.example.com;\n"));
        let secure = block(&v, "listen 443 ssl;", "*.shop.example.com");
        assert!(secure.contains("proxy_pass http://127.0.0.1:20000;"));
        assert!(!v.contains("server_name shop.example.com *.shop"));
    }

    #[test]
    fn without_a_certificate_the_vhost_serves_http_only_and_still_answers_acme() {
        let v = render_vhost(&app("ledger"), &[]);
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
    fn a_folder_process_serves_current_output_dir_with_a_spa_fallback() {
        let mut a = app("docs");
        a.processes = vec![folder("web", "dist")];
        let v = render_vhost(&a, &[]);
        assert!(v.contains("root /var/lib/ferrum/apps/docs/current/dist;"));
        assert!(v.contains("try_files $uri $uri/ /index.html;"));
        assert!(!v.contains("proxy_pass"));
    }

    #[test]
    fn a_folder_is_served_from_under_the_app_s_root_directory() {
        let mut a = app("docs");
        a.root = "apps/site".into();
        a.processes = vec![folder("web", "dist")];
        let v = render_vhost(&a, &[]);
        assert!(
            v.contains("root /var/lib/ferrum/apps/docs/current/apps/site/dist;"),
            "{v}"
        );
    }

    #[test]
    fn a_name_pointing_at_a_folder_serves_it_at_the_root_beside_the_api() {
        let mut a = app("shop");
        a.processes = vec![process("web", 20000), folder("admin", "apps/admin/dist")];
        a.routes = vec![route("/", "web", false), route("/admin", "admin", false)];
        a.domains = rows(&[
            serve("shop.example.com", "web", true),
            serve("admin.shop.example.com", "admin", false),
        ]);
        let v = render_vhost(&a, &[]);
        let shop = block(&v, "listen 80;", "shop.example.com");
        assert!(shop.contains("proxy_pass http://127.0.0.1:20000;"));
        assert!(shop.contains("location /admin/ {"));
        assert!(shop.contains("alias /var/lib/ferrum/apps/shop/current/apps/admin/dist/;"));
        assert!(shop.contains("try_files $uri $uri/ /admin/index.html;"));
        let admin = block(&v, "listen 80;", "admin.shop.example.com");
        assert!(admin.contains("root /var/lib/ferrum/apps/shop/current/apps/admin/dist;"));
        assert!(!admin.contains("location / {\n        proxy_pass"));
    }

    #[test]
    fn a_route_to_a_worker_or_an_unknown_process_renders_nothing() {
        let mut a = app("ledger");
        a.routes = vec![route("/", "web", false), route("/jobs", "jobs", false)];
        let v = render_vhost(&a, &[]);
        assert_eq!(v.matches("location /").count(), 2, "{v}");
        assert!(!v.contains("location /jobs"));
    }

    #[test]
    fn every_server_block_logs_to_the_app_s_own_files() {
        let v = render_vhost(
            &ledger_with_www(),
            &tls(&["ledger.example.com", "www.ledger.example.com"]),
        );
        let blocks = v.matches("server {").count();
        assert_eq!(blocks, 4);
        assert_eq!(
            v.matches("access_log /var/log/nginx/ferrum-ledger.access.log;")
                .count(),
            blocks
        );
        assert_eq!(
            v.matches("error_log /var/log/nginx/ferrum-ledger.error.log warn;")
                .count(),
            blocks
        );
    }

    #[test]
    fn without_a_domain_there_is_nothing_to_serve() {
        let mut a = app("ledger");
        a.domains.clear();
        let v = render_vhost(&a, &[]);
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
