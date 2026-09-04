use super::{DbError, MAINTENANCE_DB, by_name, host_error, sql};
use crate::state::State;
use ferrum_platform::Platform;
use ferrum_platform::ubuntu::PG_USER;
use std::path::{Path, PathBuf};

pub const DIR: &str = "restores";
pub const SNIFF_LEN: usize = 5;

const PGDMP: &[u8] = b"PGDMP";
const GZIP: &[u8] = &[0x1f, 0x8b];

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

/// The upload on disk; dropping it removes the file however the restore ended.
pub struct Staged {
    pub dir: PathBuf,
    pub path: PathBuf,
}

impl Staged {
    pub fn new(data_dir: &Path, database: &str) -> Self {
        let dir = dir(data_dir);
        Self {
            path: dir.join(format!("{database}.dump")),
            dir,
        }
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Drops and recreates the database under its role, then loads the dump as the cluster superuser.
pub async fn restore(
    state: &State,
    platform: &dyn Platform,
    database: &str,
    staged: &Staged,
    format: Format,
) -> anyhow::Result<()> {
    let db = by_name(state, database).await?.ok_or(DbError::NotFound)?;
    platform.chown_tree(&staged.dir, PG_USER)?;
    platform
        .postgres_sql(MAINTENANCE_DB, &sql::recreate_database(&db.name, &db.role))
        .map_err(host_error)?;
    match format {
        Format::Custom => platform.postgres_restore(&db.name, &staged.path),
        Format::Plain => platform.postgres_restore_sql(&db.name, &staged.path),
    }
    .map_err(host_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_staged_file_is_removed_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let staged = Staged::new(dir.path(), "ledger_prod");
        std::fs::create_dir_all(&staged.dir).unwrap();
        std::fs::write(&staged.path, b"PGDMP").unwrap();
        let path = staged.path.clone();
        drop(staged);
        assert!(!path.exists());
        assert!(
            path.parent().unwrap().exists(),
            "only the file goes, never the directory"
        );
    }
}
