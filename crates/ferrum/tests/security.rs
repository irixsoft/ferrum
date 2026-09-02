mod support;

use axum::http::StatusCode;
use ferrum_platform::Sshd;
use ferrum_platform::ubuntu::{APT_AUTO_UPGRADES, FAIL2BAN_JAIL_LOCAL, SSHD_DROPIN};
use support::{HOSTNAME, signed_in};

#[tokio::test]
async fn the_firewall_reads_the_ssh_port_first_and_refuses_a_second_enable() {
    let (h, cookie) = signed_in().await;
    h.platform.set_sshd(Sshd {
        port: 2222,
        password_auth: true,
    });
    let before = h.get_with_cookie("/api/security", &cookie).await;
    assert_eq!(before.status, StatusCode::OK, "{}", before.json);
    assert_eq!(before.json["firewall"]["enabled"], false);
    assert_eq!(before.json["firewall"]["ssh_port"], 2222);
    assert_eq!(before.json["bans"]["installed"], false);
    assert_eq!(before.json["updates"]["enabled"], false);
    assert_eq!(before.json["ssh"]["port"], 2222);
    assert_eq!(before.json["ssh"]["password_auth"], true);
    assert_eq!(before.json["ssh"]["keys"], serde_json::json!([]));

    let res = h
        .post_with_cookie("/api/security/firewall", "", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.json);
    let calls = h.platform.calls();
    let read = calls.iter().rposition(|c| c == "sshd_effective").unwrap();
    let apply = calls
        .iter()
        .position(|c| c == "ufw_apply 2222/tcp 80/tcp 443/tcp enable")
        .unwrap();
    assert!(read < apply, "{calls:#?}");
    let after = h.get_with_cookie("/api/security", &cookie).await;
    assert_eq!(after.json["firewall"]["enabled"], true);
    assert_eq!(after.json["firewall"]["rules"].as_array().unwrap().len(), 3);
    assert_eq!(after.json["firewall"]["rules"][0]["port"], "2222/tcp");

    let again = h
        .post_with_cookie("/api/security/firewall", "", &cookie)
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT, "{}", again.json);
    assert_eq!(h.platform.calls_matching("ufw_apply").len(), 1);

    let token = h.machine_token(true).await;
    assert_eq!(
        h.get_with_bearer("/api/security", &token).await.status,
        StatusCode::OK
    );
    for uri in [
        "/api/security/firewall",
        "/api/security/fail2ban",
        "/api/security/updates",
        "/api/security/ssh/disable-passwords",
    ] {
        let refused = h.post_with_bearer(uri, "{}", &token).await;
        assert_eq!(refused.status, StatusCode::FORBIDDEN, "{uri}");
    }
    assert!(h.platform.written(FAIL2BAN_JAIL_LOCAL).is_none());
    assert!(h.platform.written(SSHD_DROPIN).is_none());
    assert_eq!(
        h.get("/api/security").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn fail2ban_is_enabled_bans_are_listed_unbanned_and_addresses_allowlisted() {
    let (h, cookie) = signed_in().await;
    let res = h
        .post_with_cookie("/api/security/fail2ban", "", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.json);
    let jail = h.platform.written(FAIL2BAN_JAIL_LOCAL).unwrap();
    assert!(
        jail.contains("[sshd]\nenabled = true\nport = 22\n"),
        "{jail}"
    );
    assert!(jail.contains("[nginx-limit-req]"));
    assert!(
        h.platform
            .calls()
            .contains(&"service enable-now fail2ban".to_string())
    );

    h.platform.set_active("fail2ban");
    h.platform.set_jails(&["sshd", "nginx-botsearch"]);
    h.platform.ban("sshd", "45.148.10.87");
    h.platform.ban("nginx-botsearch", "45.148.10.87");
    let listed = h.get_with_cookie("/api/security", &cookie).await;
    assert_eq!(listed.json["bans"]["installed"], true);
    assert_eq!(listed.json["bans"]["jails"].as_array().unwrap().len(), 2);
    let banned = listed.json["bans"]["banned"].as_array().unwrap();
    assert_eq!(banned.len(), 2);
    assert_eq!(banned[0]["ip"], "45.148.10.87");
    assert_eq!(banned[0]["jail"], "sshd");
    assert_eq!(banned[0]["banned_at"], "2026-09-02T10:12:33Z");

    let unban = h
        .post_with_cookie("/api/security/bans/45.148.10.87/unban", "", &cookie)
        .await;
    assert_eq!(unban.status, StatusCode::NO_CONTENT, "{}", unban.json);
    assert_eq!(h.platform.calls_matching("fail2ban_unban").len(), 2);
    let missing = h
        .post_with_cookie("/api/security/bans/45.148.10.87/unban", "", &cookie)
        .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND, "{}", missing.json);

    let bad = h
        .post_with_cookie("/api/security/allowlist", r#"{"ip":"home"}"#, &cookie)
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.json);
    assert_eq!(bad.json["error"], "home is not an IP address.");
    let good = h
        .post_with_cookie(
            "/api/security/allowlist",
            r#"{"ip":"203.0.113.9"}"#,
            &cookie,
        )
        .await;
    assert_eq!(good.status, StatusCode::NO_CONTENT, "{}", good.json);
    let jail = h.platform.written(FAIL2BAN_JAIL_LOCAL).unwrap();
    assert!(
        jail.contains("ignoreip = 127.0.0.1/8 ::1 203.0.113.9\n"),
        "{jail}"
    );
    assert_eq!(
        h.platform.calls_matching("service reload fail2ban").len(),
        1
    );
    let listed = h.get_with_cookie("/api/security", &cookie).await;
    assert_eq!(
        listed.json["bans"]["allowlist"],
        serde_json::json!(["203.0.113.9"])
    );
}

#[tokio::test]
async fn updates_and_password_login_are_switched_behind_their_guards() {
    let (h, cookie) = signed_in().await;
    let res = h
        .post_with_cookie("/api/security/updates", "", &cookie)
        .await;
    assert_eq!(res.status, StatusCode::ACCEPTED, "{}", res.json);
    assert!(
        h.platform
            .written(APT_AUTO_UPGRADES)
            .unwrap()
            .contains("Unattended-Upgrade \"1\"")
    );
    assert_eq!(
        h.get_with_cookie("/api/security", &cookie).await.json["updates"]["enabled"],
        true
    );

    let wrong = h
        .post_with_cookie(
            "/api/security/ssh/disable-passwords",
            r#"{"name":"nope"}"#,
            &cookie,
        )
        .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST, "{}", wrong.json);
    assert_eq!(
        wrong.json["error"],
        format!("Type the hostname, {HOSTNAME}, to confirm.")
    );
    let body = format!(r#"{{"name":"{HOSTNAME}"}}"#);
    let no_keys = h
        .post_with_cookie("/api/security/ssh/disable-passwords", &body, &cookie)
        .await;
    assert_eq!(no_keys.status, StatusCode::BAD_REQUEST, "{}", no_keys.json);
    assert!(
        no_keys.json["error"]
            .as_str()
            .unwrap()
            .contains("/root/.ssh/authorized_keys"),
        "{}",
        no_keys.json
    );
    assert!(h.platform.written(SSHD_DROPIN).is_none());
    assert!(
        h.platform
            .calls_matching("service reload-or-restart")
            .is_empty()
    );

    h.platform.add_key("saeed@laptop");
    let listed = h.get_with_cookie("/api/security", &cookie).await;
    assert_eq!(listed.json["ssh"]["keys"][0]["comment"], "saeed@laptop");
    assert_eq!(listed.json["ssh"]["keys"][0]["kind"], "ED25519");
    let done = h
        .post_with_cookie("/api/security/ssh/disable-passwords", &body, &cookie)
        .await;
    assert_eq!(done.status, StatusCode::ACCEPTED, "{}", done.json);
    assert_eq!(
        h.platform.written(SSHD_DROPIN).as_deref(),
        Some("PasswordAuthentication no\nKbdInteractiveAuthentication no\n")
    );
    let calls = h.platform.calls();
    let tested = calls.iter().position(|c| c == "sshd_test").unwrap();
    let reloaded = calls
        .iter()
        .position(|c| c == "service reload-or-restart ssh")
        .unwrap();
    assert!(tested < reloaded);
}
