use super::Outcome;
use crate::state::State;
use crate::time;
use serde::Serialize;
use tokio::sync::broadcast;

const TOKEN_MARKER: &str = "x-access-token:";
const CAPACITY: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Line {
    pub seq: i64,
    pub at: String,
    pub stream: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum Event {
    Line { deploy_id: String, line: Line },
    Done { deploy_id: String, outcome: Outcome },
}

#[derive(Clone)]
pub struct Log {
    tx: broadcast::Sender<Event>,
}

impl Default for Log {
    fn default() -> Self {
        Self {
            tx: broadcast::channel(CAPACITY).0,
        }
    }
}

impl Log {
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    pub fn done(&self, deploy_id: &str, outcome: Outcome) {
        let _ = self.tx.send(Event::Done {
            deploy_id: deploy_id.to_string(),
            outcome,
        });
    }
}

/// Strips `x-access-token:<token>@` wherever git echoes the clone URL.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    while let Some(start) = out.find(TOKEN_MARKER) {
        let Some(end) = out[start..].find('@') else {
            break;
        };
        out.replace_range(start..start + end + 1, "");
    }
    out
}

pub async fn append(
    state: &State,
    log: &Log,
    deploy_id: &str,
    stream: &str,
    text: &str,
) -> anyhow::Result<Line> {
    let text = redact(text);
    let row = sqlx::query!(
        r#"INSERT INTO deploy_logs (deploy_id, seq, stream, line)
           VALUES (?, (SELECT coalesce(max(seq), 0) + 1 FROM deploy_logs WHERE deploy_id = ?), ?, ?)
           RETURNING seq AS "seq!", at AS "at!""#,
        deploy_id,
        deploy_id,
        stream,
        text
    )
    .fetch_one(&state.pool)
    .await?;
    let line = Line {
        seq: row.seq,
        at: time::utc(row.at),
        stream: stream.to_string(),
        text,
    };
    let _ = log.tx.send(Event::Line {
        deploy_id: deploy_id.to_string(),
        line: line.clone(),
    });
    Ok(line)
}

pub async fn lines(state: &State, deploy_id: &str, after_seq: i64) -> anyhow::Result<Vec<Line>> {
    let rows = sqlx::query!(
        r#"SELECT seq AS "seq!", at AS "at!", stream AS "stream!", line AS "line!"
           FROM deploy_logs WHERE deploy_id = ? AND seq > ? ORDER BY seq"#,
        deploy_id,
        after_seq
    )
    .fetch_all(&state.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Line {
            seq: r.seq,
            at: time::utc(r.at),
            stream: r.stream,
            text: r.line,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::{new_app, state};
    use crate::apps::{self};
    use crate::deploy::{Commit, Trigger, create};

    #[tokio::test]
    async fn the_log_redacts_the_installation_token_and_streams_live() {
        let (_d, state) = state().await;
        let log = Log::default();
        let mut live = log.subscribe();
        let app = apps::create(&state, new_app("ledger", &[("/", "main", false)]))
            .await
            .unwrap();
        let d = create(&state, &app, Trigger::Manual, "main", &Commit::default())
            .await
            .unwrap();
        append(
            &state,
            &log,
            &d.id,
            "stderr",
            "Cloning https://x-access-token:ghs_abc123@github.com/irixsoft/ledger.git",
        )
        .await
        .unwrap();
        append(&state, &log, &d.id, "stdout", "done").await.unwrap();
        let lines = lines(&state, &d.id, 0).await.unwrap();
        assert!(!lines[0].text.contains("ghs_abc123"));
        assert!(
            lines[0]
                .text
                .contains("https://github.com/irixsoft/ledger.git")
        );
        assert_eq!((lines[0].seq, lines[1].seq), (1, 2));
        assert!(lines[0].at.ends_with('Z'));
        assert_eq!(super::lines(&state, &d.id, 1).await.unwrap().len(), 1);
        match live.recv().await.unwrap() {
            Event::Line { deploy_id, line } => {
                assert_eq!(deploy_id, d.id);
                assert_eq!(line.seq, 1);
            }
            other => panic!("{other:?}"),
        }
        log.done(&d.id, Outcome::Live);
        live.recv().await.unwrap();
        assert!(matches!(live.recv().await.unwrap(), Event::Done { .. }));
    }

    #[test]
    fn every_token_in_a_line_is_removed() {
        assert_eq!(
            redact("a x-access-token:one@b and x-access-token:two@c"),
            "a b and c"
        );
        assert_eq!(redact("nothing here"), "nothing here");
        assert_eq!(redact("x-access-token:cut"), "x-access-token:cut");
    }
}
