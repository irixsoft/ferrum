mod support;

use axum::http::StatusCode;
use ed25519_dalek::Signer;
use ferrum_core::update::{self, verify};
use serde_json::Value;
use std::time::Duration;
use support::github_stub::release_json;
use support::{HOSTNAME, Harness, RELEASE_KEY, StubDownloads, StubGithub};

async fn rig() -> (Harness, String, StubGithub, StubDownloads) {
    let github = StubGithub::start().await;
    let downloads = StubDownloads::start().await;
    let h = support::harness_with_deps(&github.base, &downloads.base).await;
    ferrum_core::setup::set_hostname(&h.db, HOSTNAME)
        .await
        .unwrap();
    let link = h.enrollment("Saeed").await;
    let mut key = support::soft_passkey();
    let cookie = h.register(&mut key, &link).await.session_cookie().unwrap();
    (h, cookie, github, downloads)
}

/// Signs a release with the test key and serves its four assets; `tamper` flips one bit of
/// the signature after the sums were signed.
fn stage(
    github: &StubGithub,
    downloads: &StubDownloads,
    tag: &str,
    name: &str,
    body: &str,
    binary: &[u8],
    tamper: bool,
) {
    let digest = verify::sha256_hex(binary);
    let sums = format!(
        "{digest}  ferrum-x86_64-unknown-linux-musl\n{digest}  ferrum-aarch64-unknown-linux-musl\n"
    )
    .into_bytes();
    let mut sig = RELEASE_KEY.sign(&sums).to_bytes().to_vec();
    if tamper {
        sig[5] ^= 0x10;
    }
    for asset in [
        "ferrum-x86_64-unknown-linux-musl",
        "ferrum-aarch64-unknown-linux-musl",
    ] {
        downloads.serve(&format!("{tag}/{asset}"), binary.to_vec());
    }
    downloads.serve(&format!("{tag}/SHA256SUMS"), sums);
    downloads.serve(&format!("{tag}/SHA256SUMS.sig"), sig);
    github.set_release(release_json(
        &downloads.base,
        tag,
        name,
        body,
        binary.len() as u64,
    ));
}

async fn settled(h: &Harness, cookie: &str) -> Value {
    for _ in 0..200 {
        let res = h.get_with_cookie("/api/update", cookie).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.json);
        if res.json["running"] == false {
            return res.json;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the update never finished");
}

fn update_calls(h: &Harness) -> Vec<String> {
    h.platform
        .calls()
        .into_iter()
        .filter(|c| {
            c.starts_with("self_check")
                || c.starts_with("install_binary")
                || c.starts_with("restart_later")
        })
        .collect()
}

#[tokio::test]
async fn a_fresh_install_knows_nothing_until_it_checks_and_then_shows_what_it_found() {
    let (h, cookie, github, downloads) = rig().await;
    let fresh = h.get_with_cookie("/api/update", &cookie).await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.json);
    assert_eq!(
        fresh.json,
        serde_json::json!({
            "current": ferrum::cli::VERSION,
            "latest": null,
            "available": false,
            "checked_at": null,
            "auto": false,
            "running": false,
            "step": null,
            "error": null,
            "restarting": false,
        })
    );

    stage(
        &github,
        &downloads,
        "v0.1.4",
        "v0.1.4 (Security)",
        "## What's Changed\n\nSecurity: session cookies could be replayed.\n",
        b"new ferrum",
        false,
    );
    let checked = h.post_with_cookie("/api/update/check", "", &cookie).await;
    assert_eq!(checked.status, StatusCode::OK, "{}", checked.json);
    let latest = &checked.json["latest"];
    assert_eq!(latest["tag"], "v0.1.4");
    assert_eq!(latest["version"], "0.1.4");
    assert_eq!(latest["security"], true);
    assert_eq!(
        latest["url"],
        "https://github.com/irixsoft/ferrum/releases/tag/v0.1.4"
    );
    assert!(latest["notes"].as_str().unwrap().contains("replayed"));
    assert_eq!(latest["size_bytes"], 10);
    assert_eq!(checked.json["available"], true);
    assert!(checked.json["checked_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(checked.json["running"], false);

    let again = h.get_with_cookie("/api/update", &cookie).await;
    assert_eq!(again.json["latest"], checked.json["latest"]);
    assert_eq!(again.json["checked_at"], checked.json["checked_at"]);
    assert!(
        update_calls(&h).is_empty(),
        "a check changes nothing on the box"
    );

    let mut incomplete = release_json(&downloads.base, "v0.1.5", "v0.1.5", "", 10);
    incomplete["assets"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["name"] != "SHA256SUMS.sig");
    github.set_release(incomplete);
    let refused = h.post_with_cookie("/api/update/check", "", &cookie).await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.json);
    assert_eq!(
        refused.json["error"],
        "The release has no SHA256SUMS.sig asset."
    );
    let kept = h.get_with_cookie("/api/update", &cookie).await;
    assert_eq!(
        kept.json["latest"]["tag"], "v0.1.4",
        "a failed check keeps the last good answer"
    );

    let (offline, cookie) = support::signed_in().await;
    let unreachable = offline
        .post_with_cookie("/api/update/check", "", &cookie)
        .await;
    assert_eq!(
        unreachable.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        unreachable.json
    );
    assert!(
        unreachable.json["error"]
            .as_str()
            .unwrap()
            .starts_with("Could not check for a release: "),
        "{}",
        unreachable.json
    );
}

#[tokio::test]
async fn nothing_newer_is_a_conflict_and_a_newer_release_is_installed_in_the_background() {
    let (h, cookie, github, downloads) = rig().await;
    stage(&github, &downloads, "v0.0.1", "v0.0.1", "", b"same", false);
    let checked = h.post_with_cookie("/api/update/check", "", &cookie).await;
    assert_eq!(checked.json["available"], false, "{}", checked.json);
    let nothing = h.post_with_cookie("/api/update", "", &cookie).await;
    assert_eq!(nothing.status, StatusCode::CONFLICT, "{}", nothing.json);
    assert_eq!(
        nothing.json["error"],
        format!(
            "Ferrum {} is the latest release.",
            env!("CARGO_PKG_VERSION")
        )
    );

    stage(
        &github,
        &downloads,
        "v0.1.4",
        "v0.1.4",
        "",
        b"new ferrum",
        false,
    );
    h.post_with_cookie("/api/update/check", "", &cookie).await;
    h.platform
        .answer_self_check("ferrum 0.1.4 (build b, commit c)");
    let accepted = h.post_with_cookie("/api/update", "", &cookie).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.json);
    assert!(
        accepted.json["running"] == true || accepted.json["restarting"] == true,
        "{}",
        accepted.json
    );

    let done = settled(&h, &cookie).await;
    assert_eq!(done["restarting"], true, "{done}");
    assert_eq!(done["error"], Value::Null);
    assert_eq!(done["step"], Value::Null);
    let staged = h.data_dir().join("update").join("ferrum");
    let bin = h.data_dir().join("bin").join("ferrum");
    assert_eq!(
        update_calls(&h),
        vec![
            format!("self_check {}", staged.display()),
            format!("install_binary {} {}", staged.display(), bin.display()),
            "restart_later ferrum".to_string(),
        ]
    );
    assert_eq!(
        h.platform.installed_binary().as_deref(),
        Some(&b"new ferrum"[..])
    );
    assert!(!h.data_dir().join("update").exists());

    let twice = h.post_with_cookie("/api/update", "", &cookie).await;
    assert_eq!(twice.status, StatusCode::CONFLICT, "{}", twice.json);
    assert_eq!(
        twice.json["error"],
        "Ferrum v0.1.4 is installed and restarts in a moment."
    );
    assert_eq!(update_calls(&h).len(), 3);
}

#[tokio::test]
async fn a_tampered_signature_is_refused_and_a_good_release_can_follow() {
    let (h, cookie, github, downloads) = rig().await;
    stage(&github, &downloads, "v0.1.4", "v0.1.4", "", b"forged", true);
    h.post_with_cookie("/api/update/check", "", &cookie).await;
    let accepted = h.post_with_cookie("/api/update", "", &cookie).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.json);

    let done = settled(&h, &cookie).await;
    assert_eq!(done["error"], "The release's signature does not verify.");
    assert_eq!(done["restarting"], false);
    assert!(update_calls(&h).is_empty(), "{:?}", h.platform.calls());
    assert!(!h.data_dir().join("update").exists());

    stage(
        &github, &downloads, "v0.1.4", "v0.1.4", "", b"genuine", false,
    );
    h.post_with_cookie("/api/update/check", "", &cookie).await;
    h.platform
        .answer_self_check("ferrum 0.1.4 (build b, commit c)");
    let accepted = h.post_with_cookie("/api/update", "", &cookie).await;
    assert_eq!(accepted.status, StatusCode::ACCEPTED, "{}", accepted.json);
    let done = settled(&h, &cookie).await;
    assert_eq!(done["error"], Value::Null, "{done}");
    assert_eq!(done["restarting"], true);
    assert_eq!(
        h.platform.installed_binary().as_deref(),
        Some(&b"genuine"[..])
    );
}

#[tokio::test]
async fn the_auto_toggle_round_trips_and_a_read_only_token_can_only_look() {
    let (h, cookie, _github, _downloads) = rig().await;
    let on = h
        .put_with_cookie("/api/settings/updates", r#"{"auto":true}"#, &cookie)
        .await;
    assert_eq!(on.status, StatusCode::OK, "{}", on.json);
    assert_eq!(on.json["auto"], true);
    assert_eq!(
        h.get_with_cookie("/api/update", &cookie).await.json["auto"],
        true
    );
    assert_eq!(
        h.db.get_setting(update::AUTO_KEY).await.unwrap().as_deref(),
        Some("true")
    );
    let off = h
        .put_with_cookie("/api/settings/updates", r#"{"auto":false}"#, &cookie)
        .await;
    assert_eq!(off.json["auto"], false);

    let token = h.machine_token(true).await;
    let seen = h.get_with_bearer("/api/update", &token).await;
    assert_eq!(seen.status, StatusCode::OK, "{}", seen.json);
    for (method, uri) in [
        ("POST", "/api/update/check"),
        ("POST", "/api/update"),
        ("PUT", "/api/settings/updates"),
    ] {
        let req = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(r#"{"auto":true}"#))
            .unwrap();
        let res = h.send(req).await;
        assert_eq!(
            res.status,
            StatusCode::FORBIDDEN,
            "{method} {uri}: {}",
            res.json
        );
    }
    assert!(update_calls(&h).is_empty());
}
