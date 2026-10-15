use crate::apps::provision::app_dir;

/// Refuses an app's custom file when a directive in it, at any depth, reaches past the app.
pub fn check(text: &str, slug: &str) -> Result<(), String> {
    let parsed = parse(text);
    let home = format!("{}/", app_dir(slug).join("current").display());
    for statement in &parsed.statements {
        let name = statement[0].to_ascii_lowercase();
        if name == "disable_symlinks" {
            return Err("The custom file cannot change `disable_symlinks`.".into());
        }
        if let Some(reason) = refused(&name) {
            return Err(format!("The custom file cannot use `{name}`; {reason}"));
        }
        if matches!(name.as_str(), "root" | "alias")
            && statement[1..].iter().any(|arg| !within(arg, &home))
        {
            return Err(format!("`{name}` must stay under {home}."));
        }
    }
    if parsed.unbalanced {
        return Err("The custom file closes a block it never opened.".into());
    }
    Ok(())
}

fn refused(name: &str) -> Option<&'static str> {
    match name {
        "access_log" | "error_log" => Some("nginx's logs are Ferrum's."),
        "include" => Some("it reads files from outside the app."),
        "load_module" => Some("nginx's modules belong to the server."),
        "listen" | "server_name" => {
            Some("the app's names come from its domains under Configuration.")
        }
        _ if name.starts_with("ssl_") || name.contains("_ssl_") => Some("TLS is Ferrum's."),
        _ if name.ends_with("_store") => Some("nginx writes no files for the app."),
        _ if name.ends_with("_temp_path") || name.ends_with("_cache_path") => {
            Some("nginx's working directories are Ferrum's.")
        }
        _ if name.starts_with("lua_")
            || name.contains("_by_lua")
            || name.starts_with("perl")
            || name.starts_with("js_") =>
        {
            Some("code does not run inside nginx.")
        }
        _ => None,
    }
}

fn within(path: &str, home: &str) -> bool {
    path.starts_with(home) && !path.contains('$') && !path.split('/').any(|part| part == "..")
}

struct Parsed {
    statements: Vec<Vec<String>>,
    unbalanced: bool,
}

/// Tokenizes as nginx does: `#` and quotes open a comment or a string only where a word starts.
fn parse(text: &str) -> Parsed {
    let mut statements = Vec::new();
    let mut args: Vec<String> = Vec::new();
    let mut word: Option<Vec<u8>> = None;
    let mut quote: Option<u8> = None;
    let mut depth = 0i32;
    let mut unbalanced = false;
    let mut comment = false;
    let mut escaped = false;
    let mut variable = false;
    let mut flush = |args: &mut Vec<String>| {
        if !args.is_empty() {
            statements.push(std::mem::take(args));
        }
    };

    for &c in text.as_bytes() {
        if c == b'\n' {
            comment = false;
        }
        if comment {
            continue;
        }
        if escaped {
            escaped = false;
            if let Some(w) = word.as_mut() {
                w.push(c);
            }
            continue;
        }
        let Some(w) = word.as_mut() else {
            match c {
                b' ' | b'\t' | b'\r' | b'\n' => {}
                b';' => flush(&mut args),
                b'{' => {
                    flush(&mut args);
                    depth += 1;
                }
                b'}' => {
                    flush(&mut args);
                    depth -= 1;
                    unbalanced |= depth < 0;
                }
                b'#' => comment = true,
                b'"' | b'\'' => {
                    quote = Some(c);
                    word = Some(Vec::new());
                }
                b'\\' => {
                    escaped = true;
                    word = Some(vec![c]);
                }
                b'$' => {
                    variable = true;
                    word = Some(vec![c]);
                }
                _ => word = Some(vec![c]),
            }
            continue;
        };
        if c == b'{' && variable {
            w.push(c);
            continue;
        }
        variable = false;
        if c == b'\\' || c == b'$' {
            escaped = c == b'\\';
            variable = c == b'$';
            w.push(c);
            continue;
        }
        let ends = match quote {
            Some(q) => c == q,
            None => matches!(c, b' ' | b'\t' | b'\r' | b'\n' | b';' | b'{'),
        };
        if !ends {
            w.push(c);
            continue;
        }
        args.push(unescape(w));
        word = None;
        if quote.take().is_some() {
            continue;
        }
        if c == b';' {
            flush(&mut args);
        } else if c == b'{' {
            flush(&mut args);
            depth += 1;
        }
    }
    if let Some(w) = word {
        args.push(unescape(&w));
    }
    flush(&mut args);
    Parsed {
        statements,
        unbalanced,
    }
}

fn unescape(raw: &[u8]) -> String {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\'
            && let Some(&next) = raw.get(i + 1)
        {
            let plain = match next {
                b'"' | b'\'' | b'\\' => Some(next),
                b't' => Some(b'\t'),
                b'r' => Some(b'\r'),
                b'n' => Some(b'\n'),
                _ => None,
            };
            if let Some(byte) = plain {
                out.push(byte);
                i += 2;
                continue;
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refusal(text: &str) -> String {
        check(text, "ledger").unwrap_err()
    }

    #[test]
    fn every_directive_that_reaches_the_host_is_refused() {
        for name in [
            "access_log",
            "error_log",
            "include",
            "load_module",
            "listen",
            "server_name",
            "ssl_certificate",
            "ssl_certificate_key",
            "ssl_password_file",
            "ssl_client_certificate",
            "ssl_trusted_certificate",
            "ssl_dhparam",
            "ssl_crl",
            "ssl_stapling_file",
            "ssl_session_ticket_key",
            "ssl_conf_command",
            "proxy_ssl_certificate_key",
            "grpc_ssl_trusted_certificate",
            "proxy_store",
            "fastcgi_store",
            "uwsgi_store",
            "scgi_store",
            "client_body_temp_path",
            "proxy_temp_path",
            "proxy_cache_path",
            "fastcgi_cache_path",
            "uwsgi_cache_path",
            "scgi_cache_path",
            "lua_package_path",
            "content_by_lua_block",
            "perl_modules",
            "perl",
            "js_import",
        ] {
            let e = refusal(&format!("{name} /etc/cron.d/x;\n"));
            assert!(
                e.starts_with(&format!("The custom file cannot use `{name}`;")),
                "{e}"
            );
        }
        assert_eq!(
            refusal("access_log /etc/cron.d/x;"),
            "The custom file cannot use `access_log`; nginx's logs are Ferrum's."
        );
        assert_eq!(
            refusal("location /x {\n    disable_symlinks off;\n}\n"),
            "The custom file cannot change `disable_symlinks`."
        );
        assert!(check("auth_basic_user_file /etc/nginx/htpasswd;", "ledger").is_ok());
    }

    #[test]
    fn a_directive_is_found_at_any_depth_quoted_or_split_across_lines() {
        let nested =
            "location /x {\n    if ($arg_a) {\n        access_log /etc/cron.d/x;\n    }\n}\n";
        assert!(refusal(nested).contains("`access_log`"));
        assert!(refusal("access_log \"/etc/cron.d/x\";").contains("`access_log`"));
        assert!(refusal("'include' /etc/shadow;").contains("`include`"));
        assert!(refusal("Include\n/etc/shadow\n;").contains("`include`"));
        assert!(refusal("return 200 a#;access_log /etc/cron.d/x;").contains("`access_log`"));
        assert!(refusal("return 200 \"#\"; access_log /x;").contains("`access_log`"));
        assert!(refusal("add_header X \"a\\\"b\"; error_log /x;").contains("`error_log`"));
        assert!(refusal("} server { listen 81; }").contains("`listen`"));
        assert!(refusal("}\nlocation /x { return 418; }").contains("never opened"));
    }

    #[test]
    fn root_and_alias_stay_inside_the_release() {
        assert!(check("root /var/lib/ferrum/apps/ledger/current/public;", "ledger").is_ok());
        assert!(
            check(
                "location /d/ { alias /var/lib/ferrum/apps/ledger/current/storage/; }",
                "ledger"
            )
            .is_ok()
        );
        assert_eq!(
            refusal("root /;"),
            "`root` must stay under /var/lib/ferrum/apps/ledger/current/."
        );
        assert_eq!(
            refusal("location /x { alias /var/lib/ferrum/apps/ledger/shared/storage/; }"),
            "`alias` must stay under /var/lib/ferrum/apps/ledger/current/."
        );
        assert!(refusal("root /var/lib/ferrum/apps/notes/current/public;").contains("`root`"));
        assert!(refusal("root /var/lib/ferrum/apps/ledger/current;").contains("`root`"));
        assert!(refusal("root /var/lib/ferrum/apps/ledger/current/../shared;").contains("`root`"));
        assert!(refusal("root /var/lib/ferrum/apps/ledger/current/$arg_d;").contains("`root`"));
    }

    #[test]
    fn comments_and_everyday_directives_pass() {
        assert!(check("# access_log /etc/cron.d/x;\n", "ledger").is_ok());
        assert!(
            check(
                "client_max_body_size 50m; # include /etc/shadow;\n",
                "ledger"
            )
            .is_ok()
        );
        assert!(
            check(
                "client_max_body_size 50m;\nproxy_read_timeout 300s;\n",
                "ledger"
            )
            .is_ok()
        );
        assert!(
            check(
                "location /x {\n    return 418;\n}\nadd_header X-Frame-Options \"DENY\" always;\nset $a \"${host}x\";\n",
                "ledger"
            )
            .is_ok()
        );
        assert!(check("", "ledger").is_ok());
    }
}
