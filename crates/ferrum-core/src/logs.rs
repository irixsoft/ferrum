use crate::apps::{App, unit::unit_name};
use ferrum_platform::ubuntu::NGINX_LOG_DIR;
use ferrum_platform::{JournalLine, Platform};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::mpsc;

pub const DEFAULT_LINES: u32 = 200;
pub const MAX_LINES: u32 = 2000;
const CHANNEL: usize = 1024;
const ACCESS_STAMP: &str = "%d/%b/%Y:%H:%M:%S %z";
const ERROR_STAMP: &str = "%Y/%m/%d %H:%M:%S";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    App,
    Access,
    Error,
}

impl Source {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "app" => Some(Self::App),
            "access" => Some(Self::Access),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Line {
    pub at: String,
    pub level: &'static str,
    pub text: String,
}

pub fn access_log_path(slug: &str) -> PathBuf {
    Path::new(NGINX_LOG_DIR).join(format!("ferrum-{slug}.access.log"))
}

pub fn error_log_path(slug: &str) -> PathBuf {
    Path::new(NGINX_LOG_DIR).join(format!("ferrum-{slug}.error.log"))
}

fn stamp(at: chrono::DateTime<chrono::Utc>) -> String {
    at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn from_journal(line: JournalLine) -> Line {
    let at = chrono::DateTime::from_timestamp_micros(line.at_usec as i64)
        .map(stamp)
        .unwrap_or_default();
    let level = match line.priority {
        0..=3 => "error",
        4 => "warn",
        _ => "info",
    };
    Line {
        at,
        level,
        text: line.message,
    }
}

/// `203.0.113.44 - - [02/Sep/2026:12:05:14 +0000] "GET / HTTP/2.0" 200 4211 …`: the stamp moves
/// to its own column and the status decides the level.
pub fn from_access(text: &str) -> Line {
    let Some((before, rest)) = text.split_once(" [") else {
        return plain(text);
    };
    let Some((raw, after)) = rest.split_once("] ") else {
        return plain(text);
    };
    let at = chrono::DateTime::parse_from_str(raw, ACCESS_STAMP)
        .map(|t| stamp(t.with_timezone(&chrono::Utc)))
        .unwrap_or_default();
    let level = match status_after_request(after) {
        Some(500..=599) => "error",
        Some(400..=499) => "warn",
        _ => "info",
    };
    Line {
        at,
        level,
        text: format!("{before} {after}"),
    }
}

fn status_after_request(after: &str) -> Option<u16> {
    let mut parts = after.splitn(3, '"');
    parts.next()?;
    parts.next()?;
    parts.next()?.split_whitespace().next()?.parse().ok()
}

/// `2026/09/02 12:05:14 [error] 1234#0: *5 connect() failed …`, in the host's clock.
pub fn from_error(text: &str) -> Line {
    let Some((raw, rest)) = text.split_once(" [") else {
        return plain(text);
    };
    let Some((level_name, after)) = rest.split_once("] ") else {
        return plain(text);
    };
    let at = chrono::NaiveDateTime::parse_from_str(raw, ERROR_STAMP)
        .map(|t| stamp(t.and_utc()))
        .unwrap_or_default();
    let level = match level_name {
        "emerg" | "alert" | "crit" | "error" => "error",
        "warn" => "warn",
        _ => "info",
    };
    Line {
        at,
        level,
        text: after.to_string(),
    }
}

fn plain(text: &str) -> Line {
    Line {
        at: String::new(),
        level: "info",
        text: text.to_string(),
    }
}

pub fn tail(
    platform: &dyn Platform,
    app: &App,
    source: Source,
    lines: u32,
) -> anyhow::Result<Vec<Line>> {
    let lines = lines.clamp(1, MAX_LINES);
    Ok(match source {
        Source::App => platform
            .journal_tail(&unit_name(&app.slug), lines)?
            .into_iter()
            .map(from_journal)
            .collect(),
        Source::Access => platform
            .tail_file(&access_log_path(&app.slug), lines)?
            .iter()
            .map(|l| from_access(l))
            .collect(),
        Source::Error => platform
            .tail_file(&error_log_path(&app.slug), lines)?
            .iter()
            .map(|l| from_error(l))
            .collect(),
    })
}

/// Journald follows; dropping the receiver ends `journalctl` within a second.
pub fn follow(platform: Arc<dyn Platform>, app: &App, lines: u32) -> mpsc::Receiver<Line> {
    let (tx, rx) = mpsc::channel(CHANNEL);
    let unit = unit_name(&app.slug);
    let lines = lines.clamp(1, MAX_LINES);
    tokio::task::spawn_blocking(move || {
        let result = platform.journal_follow(
            &unit,
            lines,
            &mut |line| {
                let _ = tx.blocking_send(from_journal(line));
            },
            &|| tx.is_closed(),
        );
        if let Err(e) = result {
            tracing::warn!(unit, error = ?e, "following the journal");
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::app;
    use ferrum_platform::FakePlatform;

    #[test]
    fn journald_priorities_map_to_three_levels_and_stamps_are_utc() {
        let line = from_journal(JournalLine {
            at_usec: 1_788_348_289_288_518,
            priority: 4,
            message: "slow".into(),
        });
        assert_eq!(line.at, "2026-09-02T11:24:49Z");
        assert_eq!(line.level, "warn");
        assert_eq!(line.text, "slow");
        let error = |p| {
            from_journal(JournalLine {
                at_usec: 1,
                priority: p,
                message: String::new(),
            })
            .level
        };
        assert_eq!(error(3), "error");
        assert_eq!(error(0), "error");
        assert_eq!(error(6), "info");
        assert_eq!(error(7), "info");
    }

    #[test]
    fn an_access_line_moves_its_stamp_out_and_reads_the_status() {
        let line = from_access(
            r#"203.0.113.44 - - [02/Sep/2026:12:05:14 +0200] "GET /api/x HTTP/2.0" 503 12 "-" "curl/8""#,
        );
        assert_eq!(line.at, "2026-09-02T10:05:14Z");
        assert_eq!(line.level, "error");
        assert_eq!(
            line.text,
            r#"203.0.113.44 - - "GET /api/x HTTP/2.0" 503 12 "-" "curl/8""#
        );
        assert_eq!(
            from_access(
                r#"1.2.3.4 - - [02/Sep/2026:12:05:14 +0000] "GET / HTTP/1.1" 404 0 "-" "x""#
            )
            .level,
            "warn"
        );
        assert_eq!(
            from_access(
                r#"1.2.3.4 - - [02/Sep/2026:12:05:14 +0000] "GET / HTTP/1.1" 200 0 "-" "x""#
            )
            .level,
            "info"
        );
        let odd = from_access("not an access line");
        assert_eq!((odd.at.as_str(), odd.level), ("", "info"));
        assert_eq!(odd.text, "not an access line");
    }

    #[test]
    fn an_error_line_keeps_nginx_level_names_within_three() {
        let line = from_error("2026/09/02 12:05:14 [error] 1234#0: *5 connect() failed");
        assert_eq!(line.at, "2026-09-02T12:05:14Z");
        assert_eq!(line.level, "error");
        assert_eq!(line.text, "1234#0: *5 connect() failed");
        assert_eq!(from_error("2026/09/02 12:05:14 [warn] x").level, "warn");
        assert_eq!(from_error("2026/09/02 12:05:14 [notice] x").level, "info");
        assert_eq!(from_error("2026/09/02 12:05:14 [crit] x").level, "error");
    }

    #[test]
    fn paths_follow_the_slug_under_nginx_s_log_directory() {
        assert_eq!(
            access_log_path("ledger"),
            Path::new("/var/log/nginx/ferrum-ledger.access.log")
        );
        assert_eq!(
            error_log_path("ledger"),
            Path::new("/var/log/nginx/ferrum-ledger.error.log")
        );
        assert_eq!(Source::parse("access"), Some(Source::Access));
        assert_eq!(Source::parse("build"), None);
    }

    #[test]
    fn tail_reads_the_journal_or_the_nginx_file_the_source_names() {
        let p = FakePlatform::new();
        let a = app("ledger");
        p.journal(
            "ferrum-app-ledger",
            &[(6, "Listening"), (3, "boom"), (6, "hi\u{fffd}")],
        );
        p.write_file(
            Path::new("/var/log/nginx/ferrum-ledger.error.log"),
            "2026/09/02 12:05:14 [warn] one\n2026/09/02 12:05:15 [error] two\n",
            0o644,
        )
        .unwrap();
        let app_lines = tail(&p, &a, Source::App, 2).unwrap();
        assert_eq!(app_lines.len(), 2);
        assert_eq!(
            (app_lines[0].level, app_lines[0].text.as_str()),
            ("error", "boom")
        );
        assert!(app_lines[1].at.ends_with('Z'));
        let errors = tail(&p, &a, Source::Error, 200).unwrap();
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[1].text, "two");
        assert!(tail(&p, &a, Source::Access, 200).unwrap().is_empty());
        assert!(tail(&p, &app("nope"), Source::App, 200).unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_follow_ends_when_its_receiver_is_dropped() {
        let p = Arc::new(FakePlatform::new());
        p.journal("ferrum-app-ledger", &[(6, "one"), (6, "two")]);
        let mut rx = follow(p.clone(), &app("ledger"), 1);
        let first = rx.recv().await.unwrap();
        assert_eq!(first.text, "two");
        p.journal("ferrum-app-ledger", &[(4, "three")]);
        let live = rx.recv().await.unwrap();
        assert_eq!((live.level, live.text.as_str()), ("warn", "three"));
        drop(rx);
        for _ in 0..200 {
            if p.follows_ended() == 1 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("journal_follow kept running after the receiver was dropped");
    }
}
