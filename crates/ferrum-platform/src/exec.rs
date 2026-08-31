use crate::PlatformError;
use std::process::Command;

pub fn run(argv: &[&str]) -> Result<String, PlatformError> {
    run_env(argv, &[])
}

pub fn run_env(argv: &[&str], env: &[(&str, &str)]) -> Result<String, PlatformError> {
    let mut cmd = Command::new(argv[0]);
    cmd.args(&argv[1..]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output()?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    Err(PlatformError::Command {
        cmd: argv.join(" "),
        code: out.status.code().unwrap_or(-1),
        stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
    })
}

pub fn status(argv: &[&str]) -> bool {
    Command::new(argv[0])
        .args(&argv[1..])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
