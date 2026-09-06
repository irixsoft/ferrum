use super::{Bans, SecurityError, sshd_or_default};
use crate::state::State;
use ferrum_platform::ubuntu::{FAIL2BAN_JAIL_LOCAL, FAIL2BAN_UNIT, NGINX_LOG_DIR};
use ferrum_platform::{Ban, Platform, ServiceAction};
use std::net::IpAddr;
use std::path::Path;

pub const JAILS: [&str; 4] = [
    "sshd",
    "nginx-http-auth",
    "nginx-botsearch",
    "nginx-limit-req",
];
const ALWAYS_IGNORED: &str = "127.0.0.1/8 ::1";
const ALLOWLIST_KEY: &str = "security.allowlist";

/// The sshd jail reads the journal: Ubuntu 24.04 ships no `auth.log`. The nginx jails read
/// every error log, the global one and the per-app ones alike.
pub fn jail_local(ssh_port: u16, allowlist: &[String]) -> String {
    let mut ignore = ALWAYS_IGNORED.to_string();
    for ip in allowlist {
        ignore.push(' ');
        ignore.push_str(ip);
    }
    let mut out =
        format!("[DEFAULT]\nignoreip = {ignore}\nbantime = 1h\nfindtime = 10m\nmaxretry = 5\n\n");
    out.push_str(&format!(
        "[sshd]\nenabled = true\nport = {ssh_port}\nbackend = systemd\n"
    ));
    for jail in &JAILS[1..] {
        out.push_str(&format!(
            "\n[{jail}]\nenabled = true\nlogpath = {NGINX_LOG_DIR}/*error.log\n"
        ));
    }
    out
}

pub async fn allowlist(state: &State) -> anyhow::Result<Vec<String>> {
    Ok(state
        .get_setting(ALLOWLIST_KEY)
        .await?
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect())
}

fn write_config(platform: &dyn Platform, allowlist: &[String]) -> anyhow::Result<()> {
    let sshd = sshd_or_default(platform);
    platform.write_file(
        Path::new(FAIL2BAN_JAIL_LOCAL),
        &jail_local(sshd.port, allowlist),
        0o644,
    )?;
    Ok(())
}

/// Synchronous so a caller can put the apt run on a blocking thread; read the allowlist first.
pub fn enable(platform: &dyn Platform, allowlist: &[String]) -> anyhow::Result<()> {
    platform.install_packages(&[FAIL2BAN_UNIT])?;
    write_config(platform, allowlist)?;
    if platform.service_is_active(FAIL2BAN_UNIT) {
        platform.service(ServiceAction::Reload, FAIL2BAN_UNIT)?;
    } else {
        platform.service(ServiceAction::EnableNow, FAIL2BAN_UNIT)?;
    }
    Ok(())
}

pub async fn status(state: &State, platform: &dyn Platform) -> anyhow::Result<Bans> {
    let installed = platform.service_is_active(FAIL2BAN_UNIT);
    let (jails, banned) = if installed {
        let jails = platform.fail2ban_jails()?;
        let mut banned: Vec<Ban> = Vec::new();
        for jail in &jails {
            banned.extend(platform.fail2ban_bans(jail)?);
        }
        (jails, banned)
    } else {
        (Vec::new(), Vec::new())
    };
    Ok(Bans {
        installed,
        jails,
        banned,
        allowlist: allowlist(state).await?,
    })
}

pub fn unban(platform: &dyn Platform, ip: &str) -> anyhow::Result<()> {
    if !platform.service_is_active(FAIL2BAN_UNIT) {
        return Err(SecurityError::NotBanned(ip.to_string()).into());
    }
    let mut found = false;
    for jail in platform.fail2ban_jails()? {
        if platform.fail2ban_bans(&jail)?.iter().any(|b| b.ip == ip) {
            platform.fail2ban_unban(&jail, ip)?;
            found = true;
        }
    }
    if found {
        Ok(())
    } else {
        Err(SecurityError::NotBanned(ip.to_string()).into())
    }
}

/// Persisted in the jail file, so it survives `fail2ban-client reload` and a reboot.
pub async fn allow(state: &State, platform: &dyn Platform, ip: &str) -> anyhow::Result<()> {
    let address: IpAddr = ip
        .trim()
        .parse()
        .map_err(|_| SecurityError::BadAddress(ip.to_string()))?;
    let mut list = allowlist(state).await?;
    let entry = address.to_string();
    if !list.contains(&entry) {
        list.push(entry);
        state.set_setting(ALLOWLIST_KEY, &list.join(" ")).await?;
    }
    write_config(platform, &list)?;
    if platform.service_is_active(FAIL2BAN_UNIT) {
        platform.service(ServiceAction::Reload, FAIL2BAN_UNIT)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::tests::state;
    use ferrum_platform::{FakePlatform, Sshd};

    #[test]
    fn the_jail_file_names_the_four_jails_the_ssh_port_and_the_journal() {
        let text = jail_local(2222, &["203.0.113.9".into()]);
        for jail in JAILS {
            assert!(
                text.contains(&format!("[{jail}]\nenabled = true\n")),
                "{text}"
            );
        }
        assert!(text.contains("ignoreip = 127.0.0.1/8 ::1 203.0.113.9\n"));
        assert!(text.contains("[sshd]\nenabled = true\nport = 2222\nbackend = systemd\n"));
        assert_eq!(text.matches("backend = systemd").count(), 1);
        assert_eq!(
            text.matches("logpath = /var/log/nginx/*error.log").count(),
            3
        );
        assert!(jail_local(22, &[]).contains("ignoreip = 127.0.0.1/8 ::1\n"));
    }

    #[tokio::test]
    async fn enabling_installs_writes_and_starts_or_reloads() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        p.set_sshd(Sshd {
            port: 2222,
            password_auth: true,
        });
        assert!(!status(&state, &p).await.unwrap().installed);
        enable(&p, &allowlist(&state).await.unwrap()).unwrap();
        let calls = p.calls();
        let install = calls
            .iter()
            .position(|c| c == "install_packages fail2ban")
            .unwrap();
        let wrote = calls
            .iter()
            .position(|c| c == "write_file /etc/fail2ban/jail.d/ferrum.local 644")
            .unwrap();
        let started = calls
            .iter()
            .position(|c| c == "service enable-now fail2ban")
            .unwrap();
        assert!(install < wrote && wrote < started, "{calls:#?}");
        assert!(
            p.written(FAIL2BAN_JAIL_LOCAL)
                .unwrap()
                .contains("port = 2222")
        );
        p.set_active("fail2ban");
        enable(&p, &[]).unwrap();
        assert!(p.calls().contains(&"service reload fail2ban".to_string()));
    }

    #[tokio::test]
    async fn bans_are_listed_across_jails_and_unbanned_wherever_they_appear() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let absent = unban(&p, "45.148.10.87").unwrap_err();
        assert!(matches!(
            absent.downcast_ref::<SecurityError>(),
            Some(SecurityError::NotBanned(_))
        ));
        assert!(p.calls_matching("fail2ban").is_empty(), "nothing to ask");
        p.set_active("fail2ban");
        p.set_jails(&["sshd", "nginx-botsearch"]);
        p.ban("sshd", "45.148.10.87");
        p.ban("nginx-botsearch", "45.148.10.87");
        p.ban("nginx-botsearch", "185.220.101.4");
        let s = status(&state, &p).await.unwrap();
        assert!(s.installed);
        assert_eq!(s.jails, vec!["sshd", "nginx-botsearch"]);
        assert_eq!(s.banned.len(), 3);
        unban(&p, "45.148.10.87").unwrap();
        assert_eq!(p.calls_matching("fail2ban_unban").len(), 2);
        assert_eq!(status(&state, &p).await.unwrap().banned.len(), 1);
        let missing = unban(&p, "45.148.10.87").unwrap_err();
        assert!(matches!(
            missing.downcast_ref::<SecurityError>(),
            Some(SecurityError::NotBanned(_))
        ));
    }

    #[tokio::test]
    async fn the_allowlist_is_kept_in_settings_and_written_into_the_jail_file() {
        let (_d, state) = state().await;
        let p = FakePlatform::new();
        let bad = allow(&state, &p, "not-an-ip").await.unwrap_err();
        assert!(matches!(
            bad.downcast_ref::<SecurityError>(),
            Some(SecurityError::BadAddress(_))
        ));
        assert!(p.written(FAIL2BAN_JAIL_LOCAL).is_none());
        allow(&state, &p, " 203.0.113.9 ").await.unwrap();
        allow(&state, &p, "203.0.113.9").await.unwrap();
        allow(&state, &p, "2001:db8::1").await.unwrap();
        assert_eq!(
            allowlist(&state).await.unwrap(),
            vec!["203.0.113.9", "2001:db8::1"]
        );
        assert!(
            p.written(FAIL2BAN_JAIL_LOCAL)
                .unwrap()
                .contains("ignoreip = 127.0.0.1/8 ::1 203.0.113.9 2001:db8::1\n")
        );
        assert!(
            p.calls_matching("service reload").is_empty(),
            "not running yet"
        );
        p.set_active("fail2ban");
        allow(&state, &p, "198.51.100.7").await.unwrap();
        assert_eq!(p.calls_matching("service reload fail2ban").len(), 1);
        assert_eq!(status(&state, &p).await.unwrap().allowlist.len(), 3);
    }
}
