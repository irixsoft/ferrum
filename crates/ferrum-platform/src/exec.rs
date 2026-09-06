use crate::PlatformError;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Code(i32),
    Killed { signal: i32 },
    TimedOut,
}

impl Exit {
    pub fn success(&self) -> bool {
        matches!(self, Exit::Code(0))
    }

    fn of(status: ExitStatus) -> Self {
        match (status.code(), status.signal()) {
            (Some(code), _) => Exit::Code(code),
            (None, Some(signal)) => Exit::Killed { signal },
            (None, None) => Exit::Code(-1),
        }
    }
}

const STOP_POLL: Duration = Duration::from_secs(1);

pub struct Spawn<'a> {
    pub argv: &'a [&'a str],
    pub env: &'a [(&'a str, &'a str)],
    pub clear_env: bool,
    pub cwd: Option<&'a Path>,
    pub timeout: Option<Duration>,
    /// Run when the deadline passes and before the child is reaped, e.g. to kill a whole cgroup.
    pub on_timeout: Option<&'a [&'a str]>,
    /// Polled every second; a never-ending child such as `journalctl --follow` is killed once it answers true.
    pub stop: Option<&'a dyn Fn() -> bool>,
}

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

pub fn run_streaming(
    spawn: &Spawn<'_>,
    on_line: &mut dyn FnMut(Stream, &str),
) -> Result<Exit, PlatformError> {
    let mut cmd = Command::new(spawn.argv[0]);
    cmd.args(&spawn.argv[1..]);
    if spawn.clear_env {
        cmd.env_clear();
    }
    for (k, v) in spawn.env {
        cmd.env(k, v);
    }
    if let Some(cwd) = spawn.cwd {
        cmd.current_dir(cwd);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let (tx, rx) = mpsc::channel();
    let readers = [
        reader(
            Stream::Stdout,
            child.stdout.take().expect("stdout is piped"),
            tx.clone(),
        ),
        reader(
            Stream::Stderr,
            child.stderr.take().expect("stderr is piped"),
            tx,
        ),
    ];

    let started = Instant::now();
    let mut timed_out = false;
    let mut stopped = false;
    loop {
        let left = match spawn.timeout {
            Some(limit) => match limit.checked_sub(started.elapsed()) {
                Some(left) => Some(left),
                None => {
                    timed_out = true;
                    break;
                }
            },
            None => None,
        };
        let received = match (left, spawn.stop) {
            (None, None) => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            (Some(left), None) => rx.recv_timeout(left),
            (left, Some(stop)) => {
                let wait = left.map_or(STOP_POLL, |l| l.min(STOP_POLL));
                match rx.recv_timeout(wait) {
                    Err(RecvTimeoutError::Timeout) => {
                        if stop() {
                            stopped = true;
                            break;
                        }
                        continue;
                    }
                    other => other,
                }
            }
        };
        match received {
            Ok((stream, line)) => on_line(stream, &line),
            Err(RecvTimeoutError::Timeout) => {
                timed_out = true;
                break;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    if timed_out && let Some(kill) = spawn.on_timeout {
        let _ = run(kill);
    }
    if timed_out || stopped {
        let _ = child.kill();
    }
    let status = child.wait()?;
    if timed_out || stopped {
        for (stream, line) in rx.try_iter() {
            on_line(stream, &line);
        }
        return Ok(if timed_out {
            Exit::TimedOut
        } else {
            Exit::of(status)
        });
    }
    for handle in readers {
        let _ = handle.join();
    }
    for (stream, line) in rx.iter() {
        on_line(stream, &line);
    }
    Ok(Exit::of(status))
}

fn reader(
    stream: Stream,
    pipe: impl Read + Send + 'static,
    tx: mpsc::Sender<(Stream, String)>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end_matches(['\n', '\r']);
                    if tx.send((stream, line.to_string())).is_err() {
                        break;
                    }
                }
            }
        }
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn<'a>(argv: &'a [&'a str], timeout: Option<Duration>) -> Spawn<'a> {
        Spawn {
            argv,
            env: &[],
            clear_env: false,
            cwd: None,
            timeout,
            on_timeout: None,
            stop: None,
        }
    }

    #[test]
    fn lines_from_both_streams_arrive_and_the_exit_code_is_reported() {
        let mut seen = Vec::new();
        let exit = run_streaming(
            &spawn(&["sh", "-c", "echo out; echo err >&2; exit 3"], None),
            &mut |s, l| seen.push((s, l.to_string())),
        )
        .unwrap();
        assert_eq!(exit, Exit::Code(3));
        assert!(seen.contains(&(Stream::Stdout, "out".into())));
        assert!(seen.contains(&(Stream::Stderr, "err".into())));
    }

    #[test]
    fn a_command_past_its_deadline_is_killed_and_reported_as_timed_out() {
        let started = Instant::now();
        let mut seen = Vec::new();
        let exit = run_streaming(
            &spawn(
                &["sh", "-c", "echo started; sleep 30; true"],
                Some(Duration::from_millis(200)),
            ),
            &mut |_, l| seen.push(l.to_string()),
        )
        .unwrap();
        assert_eq!(exit, Exit::TimedOut);
        assert_eq!(seen, vec!["started"]);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the orphaned sleep still holds the pipe; the reader must not be joined"
        );
    }

    #[test]
    fn a_signal_death_is_distinguished_from_an_exit_code() {
        let exit =
            run_streaming(&spawn(&["sh", "-c", "kill -9 $$"], None), &mut |_, _| {}).unwrap();
        assert_eq!(exit, Exit::Killed { signal: 9 });
    }

    #[test]
    fn the_environment_can_be_replaced_wholesale() {
        let mut seen = Vec::new();
        let s = Spawn {
            argv: &["sh", "-c", "echo $ONLY; echo home=$HOME"],
            env: &[("ONLY", "one")],
            clear_env: true,
            cwd: None,
            timeout: None,
            on_timeout: None,
            stop: None,
        };
        run_streaming(&s, &mut |_, l| seen.push(l.to_string())).unwrap();
        assert_eq!(seen, vec!["one", "home="]);
    }

    #[test]
    fn a_never_ending_command_is_killed_once_the_stop_check_answers_true() {
        let started = Instant::now();
        let stop = move || started.elapsed() > Duration::from_millis(300);
        let mut seen = Vec::new();
        let s = Spawn {
            argv: &["sh", "-c", "echo first; sleep 30; echo never"],
            env: &[],
            clear_env: false,
            cwd: None,
            timeout: None,
            on_timeout: None,
            stop: Some(&stop),
        };
        let exit = run_streaming(&s, &mut |_, l| seen.push(l.to_string())).unwrap();
        assert_eq!(seen, vec!["first"]);
        assert_eq!(exit, Exit::Killed { signal: 9 });
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
