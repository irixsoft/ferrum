use crate::state::State;
use anyhow::Context;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

pub const SIGNATURE_HEADER: &str = "x-hub-signature-256";
pub const EVENT_HEADER: &str = "x-github-event";
pub const DELIVERY_HEADER: &str = "x-github-delivery";
const PREFIX: &str = "sha256=";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Push {
        repository: String,
        git_ref: String,
        commit_sha: String,
    },
    Release {
        repository: String,
        tag: String,
    },
    Ping,
    Other(String),
}

impl Event {
    pub fn name(&self) -> &str {
        match self {
            Event::Push { .. } => "push",
            Event::Release { .. } => "release",
            Event::Ping => "ping",
            Event::Other(name) => name,
        }
    }

    pub fn repository(&self) -> &str {
        match self {
            Event::Push { repository, .. } | Event::Release { repository, .. } => repository,
            _ => "",
        }
    }
}

/// Compares in constant time: a byte-by-byte `==` on the hex leaks the secret one guess at a time.
pub fn verify(secret: &str, signature: &str, body: &[u8]) -> bool {
    let Some(hex) = signature.strip_prefix(PREFIX) else {
        return false;
    };
    let Ok(presented) = decode_hex(hex) else {
        return false;
    };
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
        return false;
    };

    mac.update(body);
    mac.verify_slice(&presented).is_ok()
}

pub fn sign(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac takes any key");
    mac.update(body);
    let hex: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("{PREFIX}{hex}")
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, ()> {
    if !hex.len().is_multiple_of(2) {
        return Err(());
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| ()))
        .collect()
}

#[derive(Deserialize)]
struct Repository {
    full_name: String,
}

#[derive(Deserialize)]
struct PushPayload {
    repository: Repository,
    #[serde(rename = "ref")]
    git_ref: String,
    after: String,
}

#[derive(Deserialize)]
struct ReleasePayload {
    repository: Repository,
    release: ReleaseTag,
}

#[derive(Deserialize)]
struct ReleaseTag {
    tag_name: String,
}

pub fn parse(event: &str, body: &[u8]) -> anyhow::Result<Event> {
    match event {
        "push" => {
            let p: PushPayload = serde_json::from_slice(body).context("reading a push delivery")?;
            Ok(Event::Push {
                repository: p.repository.full_name,
                git_ref: p.git_ref,
                commit_sha: p.after,
            })
        }
        "release" => {
            let p: ReleasePayload =
                serde_json::from_slice(body).context("reading a release delivery")?;
            Ok(Event::Release {
                repository: p.repository.full_name,
                tag: p.release.tag_name,
            })
        }
        "ping" => Ok(Event::Ping),
        other => Ok(Event::Other(other.to_string())),
    }
}

/// Returns false when this delivery id has already been recorded — github retries, and a retry
/// must not become a second deploy.
pub async fn record(
    state: &State,
    delivery: &str,
    event: &Event,
    body: &[u8],
) -> anyhow::Result<bool> {
    let (git_ref, commit_sha) = match event {
        Event::Push {
            git_ref,
            commit_sha,
            ..
        } => (Some(git_ref.as_str()), Some(commit_sha.as_str())),
        Event::Release { tag, .. } => (Some(tag.as_str()), None),
        _ => (None, None),
    };

    let name = event.name();
    let repository = event.repository();
    let payload = String::from_utf8_lossy(body);

    let inserted = sqlx::query!(
        "INSERT INTO github_deliveries (id, event, repository, git_ref, commit_sha, payload)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(id) DO NOTHING",
        delivery,
        name,
        repository,
        git_ref,
        commit_sha,
        payload
    )
    .execute(&state.pool)
    .await?;

    Ok(inserted.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "whsec_test";

    #[test]
    fn a_signature_this_secret_produced_verifies() {
        let body = br#"{"zen":"Design for failure."}"#;
        assert!(verify(SECRET, &sign(SECRET, body), body));
    }

    #[test]
    fn a_signature_from_another_secret_is_refused() {
        let body = br#"{"zen":"Design for failure."}"#;
        assert!(!verify(SECRET, &sign("the-wrong-secret", body), body));
    }

    #[test]
    fn a_changed_body_is_refused() {
        let signature = sign(SECRET, b"original");
        assert!(!verify(SECRET, &signature, b"tampered"));
    }

    #[test]
    fn malformed_signatures_are_refused_rather_than_panicking() {
        let body = b"anything";
        for signature in [
            "",
            "deadbeef",
            "sha256=",
            "sha256=zz",
            "sha256=abc",
            "sha1=abcd",
            "sha256=00",
        ] {
            assert!(!verify(SECRET, signature, body), "accepted {signature:?}");
        }
    }

    #[test]
    fn the_signature_matches_githubs_documented_example() {
        assert_eq!(
            sign("It's a Secret to Everybody", b"Hello, World!"),
            "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17"
        );
    }

    #[test]
    fn a_push_carries_the_branch_and_the_commit() {
        let body = br#"{"ref":"refs/heads/main","after":"abc123",
                        "repository":{"full_name":"irixsoft/ledger"}}"#;
        assert_eq!(
            parse("push", body).unwrap(),
            Event::Push {
                repository: "irixsoft/ledger".into(),
                git_ref: "refs/heads/main".into(),
                commit_sha: "abc123".into(),
            }
        );
    }

    #[test]
    fn a_release_carries_the_tag() {
        let body = br#"{"release":{"tag_name":"v1.2.0"},
                        "repository":{"full_name":"irixsoft/ledger"}}"#;
        assert_eq!(
            parse("release", body).unwrap(),
            Event::Release {
                repository: "irixsoft/ledger".into(),
                tag: "v1.2.0".into(),
            }
        );
    }

    #[test]
    fn a_ping_and_an_unknown_event_do_not_fail() {
        assert_eq!(parse("ping", br#"{"zen":"x"}"#).unwrap(), Event::Ping);
        assert_eq!(
            parse("issues", b"{}").unwrap(),
            Event::Other("issues".into())
        );
    }

    #[test]
    fn a_push_that_is_not_a_push_is_an_error_not_a_panic() {
        assert!(parse("push", b"{}").is_err());
    }
}
