use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_ferrum")
}

#[test]
fn version_prints_semver_build_id_and_commit() {
    let out = Command::new(bin()).arg("version").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.starts_with("ferrum 0.0.1"), "got: {text}");
    assert!(text.contains("build "), "got: {text}");
    assert!(text.contains("commit "), "got: {text}");
}

#[test]
fn unknown_subcommand_fails() {
    let out = Command::new(bin()).arg("frobnicate").output().unwrap();
    assert!(!out.status.success());
}

#[test]
fn rollback_needs_a_slug_and_then_a_token() {
    let usage = Command::new(bin()).arg("rollback").output().unwrap();
    assert_eq!(usage.status.code(), Some(2));
    assert!(String::from_utf8(usage.stderr).unwrap().contains("<SLUG>"));

    let no_token = Command::new(bin())
        .args(["rollback", "ledger", "--restore"])
        .env_remove("FERRUM_TOKEN")
        .output()
        .unwrap();
    assert_eq!(no_token.status.code(), Some(2));
    assert!(
        String::from_utf8(no_token.stderr)
            .unwrap()
            .contains("Set FERRUM_TOKEN")
    );
}

#[test]
fn no_subcommand_prints_help_and_fails() {
    let out = Command::new(bin()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8(out.stdout).unwrap().contains("Usage:"));
}

#[test]
fn self_check_prints_the_version_line_and_touches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(bin())
        .arg("--self-check")
        .current_dir(dir.path())
        .env("FERRUM_TOKEN", "")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!(
            "ferrum {} (build {}, commit {})\n",
            ferrum::cli::VERSION,
            ferrum::cli::BUILD_ID,
            ferrum::cli::COMMIT_SHA
        )
    );
    assert!(out.stderr.is_empty());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);

    let mixed = Command::new(bin())
        .args(["--self-check", "version"])
        .output()
        .unwrap();
    assert!(!mixed.status.success(), "the flag stands alone");
}
