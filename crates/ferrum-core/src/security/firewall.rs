use super::{Firewall, SecurityError, persisted};
use ferrum_platform::ubuntu::RULES_V4;
use ferrum_platform::{Platform, Sshd};
use std::path::Path;

pub fn rules_for(ssh_port: u16) -> [String; 3] {
    [format!("{ssh_port}/tcp"), "80/tcp".into(), "443/tcp".into()]
}

pub fn status(platform: &dyn Platform, sshd: Sshd) -> anyhow::Result<Firewall> {
    let rules = platform.ufw_status()?;
    Ok(Firewall {
        enabled: rules.is_some(),
        ssh_port: sshd.port,
        rules: rules.unwrap_or_default(),
        persisted: platform.file_exists(Path::new(RULES_V4)),
    })
}

/// The SSH port is read from the running sshd first, every time; enabling with it closed
/// locks the owner out.
pub fn enable(platform: &dyn Platform) -> anyhow::Result<()> {
    if platform.ufw_status()?.is_some() {
        return Err(SecurityError::AlreadyEnabled.into());
    }
    let sshd = platform.sshd_effective()?;
    let persisted = persisted::ensure_open(platform, &[sshd.port, 80, 443])?;
    platform.install_packages(&["ufw"])?;
    let rules = rules_for(sshd.port);
    let allow: Vec<&str> = rules.iter().map(String::as_str).collect();
    platform.ufw_apply(&allow)?;
    if persisted {
        platform.iptables_flush()?;
    }
    platform.ufw_enable()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrum_platform::FakePlatform;

    fn position(calls: &[String], needle: &str) -> usize {
        calls
            .iter()
            .rposition(|c| c.starts_with(needle))
            .unwrap_or_else(|| panic!("{needle} not in {calls:#?}"))
    }

    #[test]
    fn the_ssh_port_is_read_first_and_allowed_before_the_firewall_goes_up() {
        let p = FakePlatform::new();
        p.set_sshd(Sshd {
            port: 2222,
            password_auth: true,
        });
        assert!(!status(&p, p.sshd_effective().unwrap()).unwrap().enabled);
        enable(&p).unwrap();
        let calls = p.calls();
        let read = position(&calls, "sshd_effective");
        let install = position(&calls, "install_packages ufw");
        let apply = position(&calls, "ufw_apply 2222/tcp 80/tcp 443/tcp");
        let up = position(&calls, "ufw_enable");
        assert!(
            read < install && install < apply && apply < up,
            "{calls:#?}"
        );
        assert!(
            !calls
                .iter()
                .any(|c| c.contains("22/tcp ") && !c.contains("2222/tcp"))
        );
        assert!(p.calls_matching("iptables_").is_empty(), "{calls:#?}");

        let after = status(&p, p.sshd_effective().unwrap()).unwrap();
        assert!(after.enabled);
        assert!(!after.persisted);
        assert_eq!(after.ssh_port, 2222);
        assert_eq!(after.rules.len(), 3);
        assert_eq!(after.rules[0].port, "2222/tcp");
        let again = enable(&p).unwrap_err();
        assert!(matches!(
            again.downcast_ref::<SecurityError>(),
            Some(SecurityError::AlreadyEnabled)
        ));
        assert_eq!(p.calls_matching("ufw_apply").len(), 1);
    }

    #[test]
    fn a_host_with_its_own_rules_gets_them_opened_and_flushed_before_ufw_takes_over() {
        let p = FakePlatform::new();
        p.write_file(
            Path::new(RULES_V4),
            "*filter\n-A INPUT -p tcp --dport 22 -j ACCEPT\n-A INPUT -j REJECT\nCOMMIT\n",
            0o644,
        )
        .unwrap();
        assert!(status(&p, p.sshd_effective().unwrap()).unwrap().persisted);
        enable(&p).unwrap();
        let calls = p.calls();
        let read = position(&calls, "sshd_effective");
        let wrote = position(&calls, &format!("write_file {RULES_V4}"));
        let restored = position(&calls, "iptables_restore");
        let install = position(&calls, "install_packages ufw");
        let apply = position(&calls, "ufw_apply 22/tcp 80/tcp 443/tcp");
        let flush = position(&calls, "iptables_flush");
        let up = position(&calls, "ufw_enable");
        assert!(
            read < wrote && wrote < restored && restored < install && install < apply,
            "{calls:#?}"
        );
        assert!(apply < flush && flush < up, "{calls:#?}");
        assert!(
            p.written(RULES_V4)
                .unwrap()
                .contains("--dports 80,443 -j ACCEPT\n-A INPUT -j REJECT")
        );
    }

    #[test]
    fn a_failing_sshd_read_stops_everything() {
        let p = FakePlatform::new();
        p.fail_next("sshd_effective");
        assert!(enable(&p).is_err());
        assert!(p.calls_matching("ufw_apply").is_empty());
        assert!(p.calls_matching("install_packages").is_empty());
    }
}
