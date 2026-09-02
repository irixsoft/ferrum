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

#[derive(Deserialize)]
struct Service {
    name: String,
    ok: bool,
    detail: String,
}

#[derive(Deserialize)]
struct Host {
    hostname: String,
    ferrum_version: String,
    build_id: String,
    os: String,
    arch: String,
    uptime_secs: u64,
    cpu_cores: usize,
    cpu_pct: f64,
    memory_used_mb: u64,
    memory_total_mb: u64,
    swap_used_mb: u64,
    swap_total_mb: u64,
    disk_used_gb: f64,
    disk_total_gb: f64,
    services: Vec<Service>,
}

#[derive(Deserialize)]
struct LogLine {
    at: String,
    level: String,
    text: String,
}

#[derive(Deserialize)]
struct Status {
    status: String,
}

async fn refused(res: reqwest::Response) -> anyhow::Error {
    let status = res.status();
    let refused: Option<Refused> = res.json().await.ok();
    anyhow::anyhow!(
        "{}",
        refused
            .map(|r| r.error)
            .unwrap_or_else(|| format!("the daemon answered {status}"))
    )
}

pub fn uptime(secs: u64) -> String {
    let days = secs / 86_400;
    let hours = (secs % 86_400) / 3600;
    if days > 0 {
        format!("{days}d {hours}h")
    } else {
        format!("{hours}h {}m", (secs % 3600) / 60)
    }
}

fn host_card(host: &Host) -> String {
    let mut out = format!(
        "\n  {}  {} · {} · up {}\n  ferrum {} (build {})\n\n",
        host.hostname,
        host.os,
        host.arch,
        uptime(host.uptime_secs),
        host.ferrum_version,
        host.build_id
    );
    out.push_str(&format!(
        "  cpu      {:.0}% of {} core{}\n",
        host.cpu_pct,
        host.cpu_cores,
        if host.cpu_cores == 1 { "" } else { "s" }
    ));
    out.push_str(&format!(
        "  memory   {} / {} MB\n",
        host.memory_used_mb, host.memory_total_mb
    ));
    out.push_str(&format!(
        "  swap     {} / {} MB\n",
        host.swap_used_mb, host.swap_total_mb
    ));
    out.push_str(&format!(
        "  disk     {:.1} / {:.1} GB\n\n",
        host.disk_used_gb, host.disk_total_gb
    ));
    for s in &host.services {
        out.push_str(&format!(
            "  {} {:<14} {}\n",
            if s.ok { "✓" } else { "✗" },
            s.name,
            s.detail
        ));
    }
    out
}

pub async fn status(token: &str) -> anyhow::Result<()> {
    let base = format!("http://{LISTEN_ADDR}/api");
    let res = ferrum_core::http::client()
        .get(format!("{base}/host"))
        .bearer_auth(token)
        .send()
        .await
        .context("reaching the Ferrum daemon on loopback")?;
    if !res.status().is_success() {
        return Err(refused(res).await);
    }
    let host: Host = res.json().await?;
    print!("{}", host_card(&host));
    Ok(())
}

fn print_line(line: &LogLine) {
    let clock = line.at.get(11..19).unwrap_or("        ");
    println!("  {clock}  {:<5} {}", line.level, line.text);
}

pub async fn logs(
    slug: &str,
    source: &str,
    follow: bool,
    lines: u32,
    token: &str,
) -> anyhow::Result<()> {
    let base = format!("http://{LISTEN_ADDR}/api");
    let http = ferrum_core::http::client();
    let url = format!("{base}/apps/{slug}/logs?source={source}&lines={lines}");
    if !follow {
        let res = http
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .context("reaching the Ferrum daemon on loopback")?;
        if !res.status().is_success() {
            return Err(refused(res).await);
        }
        for line in res.json::<Vec<LogLine>>().await? {
            print_line(&line);
        }
        return Ok(());
    }
    let mut res = http
        .get(format!("{url}&follow=1"))
        .bearer_auth(token)
        .header("accept", "text/event-stream")
        .send()
        .await
        .context("reaching the Ferrum daemon on loopback")?;
    if !res.status().is_success() {
        return Err(refused(res).await);
    }
    let mut buffer = String::new();
    while let Some(chunk) = res.chunk().await? {
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        for (event, data) in frames(&mut buffer) {
            if event == "line"
                && let Ok(line) = serde_json::from_str::<LogLine>(&data)
            {
                print_line(&line);
            }
        }
    }
    Ok(())
}

pub async fn restart(slug: &str, token: &str) -> anyhow::Result<()> {
    let base = format!("http://{LISTEN_ADDR}/api");
    let http = ferrum_core::http::client();
    let res = http
        .post(format!("{base}/apps/{slug}/restart"))
        .bearer_auth(token)
        .send()
        .await
        .context("reaching the Ferrum daemon on loopback")?;
    if !res.status().is_success() {
        return Err(refused(res).await);
    }
    let shown: Status = http
        .get(format!("{base}/apps/{slug}"))
        .bearer_auth(token)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    println!("\n  Restarted {slug}; it is {}.\n", shown.status);
    Ok(())
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

    #[test]
    fn the_host_card_reads_like_the_dashboard() {
        let host = Host {
            hostname: "panel.example.com".into(),
            ferrum_version: "0.0.1".into(),
            build_id: "2026.09.02-abc".into(),
            os: "Ubuntu 24.04 LTS".into(),
            arch: "x86_64".into(),
            uptime_secs: 86_400 * 41 + 3600 * 7,
            cpu_cores: 2,
            cpu_pct: 38.4,
            memory_used_mb: 2712,
            memory_total_mb: 3934,
            swap_used_mb: 208,
            swap_total_mb: 2048,
            disk_used_gb: 21.4,
            disk_total_gb: 76.0,
            services: vec![
                Service {
                    name: "nginx".into(),
                    ok: true,
                    detail: "active".into(),
                },
                Service {
                    name: "Certificates".into(),
                    ok: false,
                    detail: "ledger.example.com renews in 12 days".into(),
                },
            ],
        };
        let card = host_card(&host);
        assert!(card.contains("panel.example.com  Ubuntu 24.04 LTS · x86_64 · up 41d 7h"));
        assert!(card.contains("cpu      38% of 2 cores"));
        assert!(card.contains("disk     21.4 / 76.0 GB"));
        assert!(card.contains("✓ nginx          active"));
        assert!(card.contains("✗ Certificates   ledger.example.com renews in 12 days"));
        assert_eq!(uptime(3_725), "1h 2m");
    }
}
