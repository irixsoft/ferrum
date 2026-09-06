use super::{Database, DbError, host_error, sql};
use ferrum_platform::Platform;
use ferrum_platform::ubuntu::PG_USER;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub const DIR: &str = "restores";
pub const SNIFF_LEN: usize = 5;

const PGDMP: &[u8] = b"PGDMP";
const GZIP: &[u8] = &[0x1f, 0x8b];
const SKIPPED: [&[u8]; 6] = [
    b"GRANT ",
    b"REVOKE ",
    b"ALTER DEFAULT PRIVILEGES",
    b"SET SESSION AUTHORIZATION",
    b"SET ROLE",
    b"COMMENT ON EXTENSION ",
];
const CREATE_EXTENSION: &[u8] = b"CREATE EXTENSION ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Custom,
    Plain,
}

pub fn sniff(head: &[u8]) -> Result<Format, DbError> {
    if head.starts_with(PGDMP) {
        return Ok(Format::Custom);
    }
    if head.starts_with(GZIP) {
        return Err(DbError::Invalid(
            "That is a gzip stream. Ferrum restores what pg_dump wrote; gunzip it first.".into(),
        ));
    }
    if head.is_empty() {
        return Err(DbError::Invalid("The upload was empty.".into()));
    }
    Ok(Format::Plain)
}

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR)
}

/// The upload on disk; dropping it removes the file and its list however the load ended.
pub struct Staged {
    pub dir: PathBuf,
    pub path: PathBuf,
    pub list: PathBuf,
}

impl Staged {
    pub fn new(data_dir: &Path, database: &str) -> Self {
        let dir = dir(data_dir);
        Self {
            path: dir.join(format!("{database}.dump")),
            list: dir.join(format!("{database}.list")),
            dir,
        }
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(&self.list);
    }
}

/// A plain dump made to load as `role`: it opens with `SET ROLE`; the ownership, privilege and
/// extension statements are left out and the extension names returned, since the superuser
/// creates those first. Lines inside COPY data are copied byte for byte.
pub fn rewrite_plain(
    mut input: impl BufRead,
    mut output: impl Write,
    role: &str,
) -> std::io::Result<Vec<String>> {
    writeln!(output, "SET ROLE {};", sql::quote_ident(role))?;
    let mut extensions = Vec::new();
    let mut line = Vec::new();
    let mut in_data = false;
    loop {
        line.clear();
        if input.read_until(b'\n', &mut line)? == 0 {
            output.flush()?;
            return Ok(extensions);
        }
        let text = trim_end(&line);
        if in_data {
            in_data = text != b"\\.";
        } else if text.starts_with(b"COPY ") && text.ends_with(b"FROM stdin;") {
            in_data = true;
        } else if let Some(rest) = text.strip_prefix(CREATE_EXTENSION) {
            if let Some(name) = extension_name(rest) {
                extensions.push(name);
            }
            continue;
        } else if skipped(text) {
            continue;
        }
        output.write_all(&line)?;
    }
}

fn trim_end(line: &[u8]) -> &[u8] {
    let end = line
        .iter()
        .rposition(|b| *b != b'\n' && *b != b'\r')
        .map_or(0, |i| i + 1);
    &line[..end]
}

fn skipped(text: &[u8]) -> bool {
    (text.starts_with(b"ALTER ") && text.windows(10).any(|w| w == b" OWNER TO "))
        || SKIPPED.iter().any(|p| text.starts_with(p))
}

/// The name after `CREATE EXTENSION [IF NOT EXISTS]`, unquoted.
fn extension_name(rest: &[u8]) -> Option<String> {
    let rest = String::from_utf8_lossy(rest);
    let rest = rest.trim_start();
    let rest = rest
        .strip_prefix("IF NOT EXISTS ")
        .unwrap_or(rest)
        .trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        let end = quoted.find('"')?;
        return Some(quoted[..end].replace("\"\"", "\""));
    }
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ';')
        .unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].to_string())
}

/// `pg_restore -l` output with the extension entries commented out, and their names. The
/// comment entries would fail as the role once the superuser owns the extension.
pub fn filter_list(listing: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(listing.len() + 16);
    let mut extensions = Vec::new();
    for line in listing.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let entry = match words.as_slice() {
            [_, _, _, "EXTENSION", "-", name, ..] => Some(*name),
            [_, _, _, "COMMENT", "-", "EXTENSION", name, ..] => Some(*name),
            _ => None,
        };
        if let Some(name) = entry {
            if !extensions.iter().any(|e| e == name) {
                extensions.push(name.to_string());
            }
            out.push(';');
        }
        out.push_str(line);
        out.push('\n');
    }
    (out, extensions)
}

/// What the dump needs, and where its edited table of contents is.
pub struct Prepared {
    pub extensions: Vec<String>,
    pub list: Option<PathBuf>,
}

pub fn prepare(
    platform: &dyn Platform,
    staged: &Staged,
    format: Format,
    role: &str,
) -> anyhow::Result<Prepared> {
    match format {
        Format::Plain => {
            let tmp = staged.path.with_extension("tmp");
            let input = std::io::BufReader::new(std::fs::File::open(&staged.path)?);
            let output = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            let extensions = rewrite_plain(input, output, role)?;
            std::fs::rename(&tmp, &staged.path)?;
            Ok(Prepared {
                extensions,
                list: None,
            })
        }
        Format::Custom => {
            let listing = platform
                .postgres_restore_list(&staged.path)
                .map_err(host_error)?;
            let (list, extensions) = filter_list(&listing);
            std::fs::write(&staged.list, list)?;
            Ok(Prepared {
                extensions,
                list: Some(staged.list.clone()),
            })
        }
    }
}

/// Loads the prepared dump into the freshly created database as its role, so every object it
/// creates is owned by that role.
pub fn load(
    platform: &dyn Platform,
    db: &Database,
    staged: &Staged,
    format: Format,
    list: Option<&Path>,
) -> anyhow::Result<()> {
    platform.chown_tree(&staged.dir, PG_USER)?;
    match format {
        Format::Custom => platform.postgres_restore(&db.name, &staged.path, Some(&db.role), list),
        Format::Plain => platform.postgres_restore_sql(&db.name, &staged.path),
    }
    .map_err(host_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrum_platform::FakePlatform;

    #[test]
    fn the_custom_format_announces_itself() {
        assert_eq!(sniff(b"PGDMP\x01\x0e\x00").unwrap(), Format::Custom);
    }

    #[test]
    fn anything_else_is_plain_sql_even_when_short() {
        assert_eq!(
            sniff(b"--\n-- PostgreSQL database dump\n").unwrap(),
            Format::Plain
        );
        assert_eq!(sniff(b"PGD").unwrap(), Format::Plain);
    }

    #[test]
    fn a_gzip_stream_and_an_empty_upload_are_refused_with_a_sentence() {
        let gzip = sniff(&[0x1f, 0x8b, 0x08, 0x00]).unwrap_err().to_string();
        assert!(gzip.contains("gunzip"), "{gzip}");
        let empty = sniff(b"").unwrap_err().to_string();
        assert!(empty.contains("empty"), "{empty}");
    }

    #[test]
    fn the_staged_files_are_removed_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let staged = Staged::new(dir.path(), "ledger_prod");
        std::fs::create_dir_all(&staged.dir).unwrap();
        std::fs::write(&staged.path, b"PGDMP").unwrap();
        std::fs::write(&staged.list, b";1\n").unwrap();
        let (path, list) = (staged.path.clone(), staged.list.clone());
        drop(staged);
        assert!(!path.exists());
        assert!(!list.exists());
        assert!(
            path.parent().unwrap().exists(),
            "only the files go, never the directory"
        );
    }

    #[test]
    fn a_plain_dump_loses_its_owners_privileges_and_extensions_but_never_a_data_line() {
        let dump = b"SET statement_timeout = 0;\r\n\
SET SESSION AUTHORIZATION dbadmin;\n\
CREATE SCHEMA audit;\n\
ALTER SCHEMA audit OWNER TO dbadmin;\n\
CREATE EXTENSION IF NOT EXISTS citext WITH SCHEMA public;\n\
COMMENT ON EXTENSION citext IS 'data type for case-insensitive character strings';\n\
CREATE EXTENSION \"uuid-ossp\" WITH SCHEMA public;\n\
CREATE EXTENSION IF NOT EXISTS citext;\n\
CREATE TABLE public.users (id integer NOT NULL, note text);\n\
ALTER TABLE public.users OWNER TO dbadmin;\n\
ALTER SEQUENCE public.users_id_seq OWNER TO dbadmin;\n\
ALTER FUNCTION audit.count_users() OWNER TO \"db admin\";\n\
COPY public.users (id, note) FROM stdin;\n\
1\tALTER TABLE public.users OWNER TO nobody;\n\
2\tCREATE EXTENSION postgis;\n\
3\t\xff\xfe not utf-8\n\
\\.\n\
GRANT SELECT ON TABLE public.users TO reader;\n\
REVOKE ALL ON SCHEMA public FROM PUBLIC;\n\
ALTER DEFAULT PRIVILEGES FOR ROLE dbadmin IN SCHEMA public GRANT SELECT ON TABLES TO reader;\n\
ALTER TABLE ONLY public.users ADD CONSTRAINT users_pkey PRIMARY KEY (id);\n";
        let mut out = Vec::new();
        let extensions = rewrite_plain(&dump[..], &mut out, "ledger_prod").unwrap();
        assert_eq!(extensions, vec!["citext", "uuid-ossp", "citext"]);
        let expected: &[u8] = b"SET ROLE \"ledger_prod\";\n\
SET statement_timeout = 0;\r\n\
CREATE SCHEMA audit;\n\
CREATE TABLE public.users (id integer NOT NULL, note text);\n\
COPY public.users (id, note) FROM stdin;\n\
1\tALTER TABLE public.users OWNER TO nobody;\n\
2\tCREATE EXTENSION postgis;\n\
3\t\xff\xfe not utf-8\n\
\\.\n\
ALTER TABLE ONLY public.users ADD CONSTRAINT users_pkey PRIMARY KEY (id);\n";
        assert_eq!(
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(expected)
        );
    }

    #[test]
    fn a_table_of_contents_keeps_everything_but_its_extension_entries() {
        let listing = ";\n; Archive created at Mon Sep  6 2026\n;\n\
2; 3079 16387 EXTENSION - citext \n\
3584; 0 0 COMMENT - EXTENSION citext \n\
4; 3079 16400 EXTENSION - vector \n\
6; 2615 16391 SCHEMA - audit dbadmin\n\
3585; 0 0 COMMENT - SCHEMA audit dbadmin\n\
218; 1259 16392 TABLE public users dbadmin\n";
        let (list, extensions) = filter_list(listing);
        assert_eq!(extensions, vec!["citext", "vector"]);
        assert_eq!(
            list,
            ";\n; Archive created at Mon Sep  6 2026\n;\n\
;2; 3079 16387 EXTENSION - citext \n\
;3584; 0 0 COMMENT - EXTENSION citext \n\
;4; 3079 16400 EXTENSION - vector \n\
6; 2615 16391 SCHEMA - audit dbadmin\n\
3585; 0 0 COMMENT - SCHEMA audit dbadmin\n\
218; 1259 16392 TABLE public users dbadmin\n"
        );
    }

    fn database() -> Database {
        Database {
            id: "id".into(),
            name: "ledger_prod".into(),
            role: "ledger_prod".into(),
            connection_limit: 20,
            extensions: vec![],
            linked_apps: vec![],
            size_bytes: None,
            connections_active: None,
            created_at: String::new(),
        }
    }

    #[test]
    fn a_plain_dump_is_rewritten_in_place_then_loaded_through_psql() {
        let dir = tempfile::tempdir().unwrap();
        let staged = Staged::new(dir.path(), "ledger_prod");
        std::fs::create_dir_all(&staged.dir).unwrap();
        std::fs::write(
            &staged.path,
            "CREATE EXTENSION IF NOT EXISTS citext;\nCREATE TABLE t (id int);\nALTER TABLE public.t OWNER TO dbadmin;\n",
        )
        .unwrap();
        let p = FakePlatform::new();
        let prepared = prepare(&p, &staged, Format::Plain, "ledger_prod").unwrap();
        assert_eq!(prepared.extensions, vec!["citext"]);
        assert!(prepared.list.is_none());
        assert_eq!(
            std::fs::read_to_string(&staged.path).unwrap(),
            "SET ROLE \"ledger_prod\";\nCREATE TABLE t (id int);\n"
        );
        assert!(!staged.path.with_extension("tmp").exists());

        load(&p, &database(), &staged, Format::Plain, None).unwrap();
        let calls = p.calls();
        let chown = calls
            .iter()
            .position(|c| c.starts_with("chown_tree"))
            .unwrap();
        let restore = calls
            .iter()
            .position(|c| c.starts_with("postgres_restore_sql ledger_prod "))
            .unwrap();
        assert!(chown < restore, "{calls:#?}");
    }

    #[test]
    fn a_custom_dump_gets_a_filtered_list_and_loads_as_the_role() {
        let dir = tempfile::tempdir().unwrap();
        let staged = Staged::new(dir.path(), "ledger_prod");
        std::fs::create_dir_all(&staged.dir).unwrap();
        std::fs::write(&staged.path, b"PGDMP\x01").unwrap();
        let p = FakePlatform::new();
        p.answer_restore_list("2; 3079 16387 EXTENSION - vector \n5; 0 0 TABLE public t dbadmin\n");
        let prepared = prepare(&p, &staged, Format::Custom, "ledger_prod").unwrap();
        assert_eq!(prepared.extensions, vec!["vector"]);
        assert_eq!(prepared.list.as_deref(), Some(staged.list.as_path()));
        assert_eq!(
            std::fs::read_to_string(&staged.list).unwrap(),
            ";2; 3079 16387 EXTENSION - vector \n5; 0 0 TABLE public t dbadmin\n"
        );
        assert_eq!(std::fs::read(&staged.path).unwrap(), b"PGDMP\x01");

        load(
            &p,
            &database(),
            &staged,
            Format::Custom,
            prepared.list.as_deref(),
        )
        .unwrap();
        assert_eq!(
            p.calls_matching("postgres_restore ledger_prod "),
            vec![format!(
                "postgres_restore ledger_prod {} ledger_prod {}",
                staged.path.display(),
                staged.list.display()
            )]
        );
    }
}
