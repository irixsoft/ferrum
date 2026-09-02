use crate::PlatformError;
use std::io::Write;
use std::process::{Command, Output, Stdio};

pub fn run(argv: &[&str]) -> Result<String, PlatformError> {
    run_env(argv, &[])
}

pub fn run_env(argv: &[&str], env: &[(&str, &str)]) -> Result<String, PlatformError> {
    let mut cmd = Command::new(argv[0]);
    cmd.args(&argv[1..]);
    for (k, v) in env {
        cmd.env(k, v);
    }
    finish(argv, cmd.output()?)
}

pub fn run_with_stdin(argv: &[&str], input: &str) -> Result<String, PlatformError> {
    let mut child = Command::new(argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = input.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let out = child.wait_with_output()?;
    writer.join().expect("stdin writer panicked")?;
    finish(argv, out)
}

fn finish(argv: &[&str], out: Output) -> Result<String, PlatformError> {
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
