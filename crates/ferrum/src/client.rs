use anyhow::{Context, bail};
use ferrum_core::LISTEN_ADDR;
use serde::Deserialize;

#[derive(Deserialize)]
struct Queued {
    id: String,
    commit_sha: Option<String>,
    git_ref: String,
    queue_position: Option<u32>,
}

#[derive(Deserialize)]
struct Refused {
    error: String,
}

#[derive(Deserialize)]
struct Line {
    stream: String,
    text: String,
}

#[derive(Deserialize)]
struct Done {
    outcome: String,
}

/// One SSE frame: the event name and its joined `data:` lines.
pub fn frames(buffer: &mut String) -> Vec<(String, String)> {
    let mut out = Vec::new();
    while let Some(end) = buffer.find("\n\n") {
        let frame = buffer[..end].to_string();
        buffer.replace_range(..end + 2, "");
        let mut event = String::from("message");
        let mut data = Vec::new();
        for line in frame.lines() {
            if let Some(name) = line.strip_prefix("event:") {
                event = name.trim().to_string();
            } else if let Some(text) = line.strip_prefix("data:") {
                data.push(text.strip_prefix(' ').unwrap_or(text).to_string());
            }
        }
        if !data.is_empty() {
            out.push((event, data.join("\n")));
        }
    }
    out
}

/// Talks to the daemon over loopback with a bearer token, prints the log, and returns the exit
/// code: 0 when the deploy ends live, 1 otherwise.
pub async fn deploy(slug: &str, git_ref: Option<&str>, token: &str) -> anyhow::Result<i32> {
    let base = format!("http://{LISTEN_ADDR}/api");
    let http = ferrum_core::http::client();
    let body = serde_json::json!({ "ref": git_ref, "cli": true });
    let res = http
        .post(format!("{base}/apps/{slug}/deploys"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .context("reaching the Ferrum daemon on loopback")?;
    if !res.status().is_success() {
        let status = res.status();
        let refused: Option<Refused> = res.json().await.ok();
        bail!(
            "{}",
            refused
                .map(|r| r.error)
                .unwrap_or_else(|| format!("the daemon answered {status}"))
        );
    }
    let queued: Queued = res.json().await?;
    match queued.queue_position {
        Some(ahead) if ahead > 0 => println!(
            "  Queued {} at {} behind {ahead} other deploy(s).",
            queued.git_ref,
            short(queued.commit_sha.as_deref())
        ),
        _ => println!(
            "  Deploying {} at {}.",
            queued.git_ref,
            short(queued.commit_sha.as_deref())
        ),
    }

    let mut res = http
        .get(format!("{base}/deploys/{}/log", queued.id))
        .bearer_auth(token)
        .header("accept", "text/event-stream")
        .send()
        .await?
        .error_for_status()
        .context("following the deploy log")?;
    let mut buffer = String::new();
    while let Some(chunk) = res.chunk().await? {
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        for (event, data) in frames(&mut buffer) {
            match event.as_str() {
                "line" => {
                    if let Ok(line) = serde_json::from_str::<Line>(&data) {
                        if line.stream == "system" {
                            println!("  {}", line.text);
                        } else {
                            println!("    {}", line.text);
                        }
                    }
                }
                "done" => {
                    let done: Done = serde_json::from_str(&data)?;
                    println!("\n  {}\n", done.outcome);
                    return Ok(if done.outcome == "Live" { 0 } else { 1 });
                }
                _ => {}
            }
        }
    }
    bail!("the log ended before the deploy did")
}

fn short(sha: Option<&str>) -> String {
    sha.map(|s| s.chars().take(7).collect())
        .unwrap_or_else(|| "HEAD".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_frames_are_split_on_blank_lines_and_keep_partial_input() {
        let mut buffer = String::from(
            "event: line\ndata: {\"a\":1}\n\nevent: done\ndata: {\"b\":2}\n\n: keep-alive\n\nevent: line\ndata: par",
        );
        let got = frames(&mut buffer);
        assert_eq!(
            got,
            vec![
                ("line".to_string(), "{\"a\":1}".to_string()),
                ("done".to_string(), "{\"b\":2}".to_string())
            ]
        );
        assert_eq!(buffer, "event: line\ndata: par");
    }
}
