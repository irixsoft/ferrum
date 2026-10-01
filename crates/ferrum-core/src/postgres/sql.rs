pub fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

pub fn quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub fn create_role(role: &str, password: &str, limit: u32) -> String {
    format!(
        "CREATE ROLE {} LOGIN PASSWORD {} CONNECTION LIMIT {limit};\n",
        quote_ident(role),
        quote_literal(password)
    )
}

pub fn create_database(name: &str, role: &str) -> String {
    format!(
        "CREATE DATABASE {} OWNER {};\n",
        quote_ident(name),
        quote_ident(role)
    )
}

pub fn isolate(name: &str, role: &str) -> String {
    format!(
        "REVOKE CONNECT ON DATABASE {name} FROM PUBLIC;\nGRANT CONNECT ON DATABASE {name} TO {role};\n",
        name = quote_ident(name),
        role = quote_ident(role)
    )
}

pub fn create_extension(extension: &str) -> String {
    format!(
        "CREATE EXTENSION IF NOT EXISTS {};\n",
        quote_ident(extension)
    )
}

/// An empty database under the same role, ready for a restore; the role and its password survive.
pub fn recreate_database(name: &str, role: &str) -> String {
    format!(
        "DROP DATABASE IF EXISTS {} WITH (FORCE);\n{}{}",
        quote_ident(name),
        create_database(name, role),
        isolate(name, role)
    )
}

pub fn drop_database(name: &str, roles: &[&str]) -> String {
    let mut out = format!(
        "DROP DATABASE IF EXISTS {} WITH (FORCE);\n",
        quote_ident(name)
    );
    for role in roles {
        out.push_str(&format!("DROP ROLE IF EXISTS {};\n", quote_ident(role)));
    }
    out
}

pub fn grant_connect(name: &str, role: &str) -> String {
    format!(
        "GRANT CONNECT ON DATABASE {} TO {};\n",
        quote_ident(name),
        quote_ident(role)
    )
}

pub fn alter_password(role: &str, password: &str) -> String {
    format!(
        "ALTER ROLE {} PASSWORD {};\n",
        quote_ident(role),
        quote_literal(password)
    )
}

/// Run inside the role's database: what it created passes to the owner, its grants go with it.
pub fn drop_role(role: &str, owner: &str) -> String {
    format!(
        "REASSIGN OWNED BY {role} TO {owner};\nDROP OWNED BY {role};\nDROP ROLE IF EXISTS {role};\n",
        role = quote_ident(role),
        owner = quote_ident(owner)
    )
}

pub fn available_extensions() -> String {
    "SELECT name FROM pg_available_extensions ORDER BY name;\n".to_string()
}

pub fn sizes() -> String {
    "SELECT d.datname, pg_database_size(d.datname), \
     (SELECT count(*) FROM pg_stat_activity a WHERE a.datname = d.datname) \
     FROM pg_database d WHERE NOT d.datistemplate;\n"
        .to_string()
}

/// psql's `-A -t` output: one `name|bytes|connections` line per database.
pub fn parse_sizes(output: &str) -> Vec<(String, i64, i64)> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('|');
            let name = parts.next()?.to_string();
            let bytes = parts.next()?.trim().parse().ok()?;
            let connections = parts.next()?.trim().parse().ok()?;
            Some((name, bytes, connections))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_and_literals_cannot_break_out() {
        assert_eq!(quote_ident(r#"a"b"#), r#""a""b""#);
        assert_eq!(quote_literal("it's"), "'it''s'");
        assert_eq!(
            create_role("x", "p'w", 5),
            "CREATE ROLE \"x\" LOGIN PASSWORD 'p''w' CONNECTION LIMIT 5;\n"
        );
        assert_eq!(
            alter_password("x", "p'w"),
            "ALTER ROLE \"x\" PASSWORD 'p''w';\n"
        );
    }

    #[test]
    fn a_database_drops_with_every_role_and_a_role_leaves_nothing_behind() {
        assert_eq!(
            drop_database("ledger", &["ledger", "ledger_app"]),
            "DROP DATABASE IF EXISTS \"ledger\" WITH (FORCE);\nDROP ROLE IF EXISTS \"ledger\";\nDROP ROLE IF EXISTS \"ledger_app\";\n"
        );
        assert_eq!(
            drop_role("ledger_app", "ledger"),
            "REASSIGN OWNED BY \"ledger_app\" TO \"ledger\";\nDROP OWNED BY \"ledger_app\";\nDROP ROLE IF EXISTS \"ledger_app\";\n"
        );
    }

    #[test]
    fn the_size_query_output_is_parsed_and_junk_is_skipped() {
        let out = "postgres|7000000|1\nledger_prod|123456|3\n\nnot a row\n";
        assert_eq!(
            parse_sizes(out),
            vec![
                ("postgres".to_string(), 7_000_000, 1),
                ("ledger_prod".to_string(), 123_456, 3)
            ]
        );
    }
}
