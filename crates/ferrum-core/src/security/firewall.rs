use super::{Firewall, SecurityError};
use ferrum_platform::{Platform, Sshd};

pub fn rules_for(ssh_port: u16) -> [String; 3] {
    [format!("{ssh_port}/tcp"), "80/tcp".into(), "443/tcp".into()]
}

pub fn status(platform: &dyn Platform, sshd: Sshd) -> anyhow::Result<Firewall> {
    let rules = platform.ufw_status()?;
    Ok(Firewall {
        enabled: rules.is_some(),
        ssh_port: sshd.port,
        rules: rules.unwrap_or_default(),
    })
}

/// The SSH port is read from the running sshd first, every time; enabling with it closed
/// locks the owner out.
pub fn enable(platform: &dyn Platform) -> anyhow::Result<()> {
    if platform.ufw_status()?.is_some() {
        return Err(SecurityError::AlreadyEnabled.into());
    }
    let sshd = platform.sshd_effective()?;
    platform.install_packages(&["ufw"])?;
    let rules = rules_for(sshd.port);
    let allow: Vec<&str> = rules.iter().map(String::as_str).collect();
    platform.ufw_apply(&allow, true)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrum_platform::FakePlatform;

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
        let read = calls.iter().position(|c| c == "sshd_effective").unwrap();
        let install = calls
            .iter()
            .position(|c| c == "install_packages ufw")
            .unwrap();
        let apply = calls
            .iter()
            .position(|c| c == "ufw_apply 2222/tcp 80/tcp 443/tcp enable")
            .unwrap();
        assert!(read < install && install < apply, "{calls:#?}");
        assert!(
            !calls
                .iter()
                .any(|c| c.contains("22/tcp ") && !c.contains("2222/tcp"))
        );

        let after = status(&p, p.sshd_effective().unwrap()).unwrap();
        assert!(after.enabled);
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
    fn a_failing_sshd_read_stops_everything() {
        let p = FakePlatform::new();
        p.fail_next("sshd_effective");
        assert!(enable(&p).is_err());
        assert!(p.calls_matching("ufw_apply").is_empty());
        assert!(p.calls_matching("install_packages").is_empty());
    }
}
