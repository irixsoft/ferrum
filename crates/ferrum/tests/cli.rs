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
