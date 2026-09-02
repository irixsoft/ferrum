use super::{SecurityError, Ssh};
use ferrum_platform::ubuntu::{SSH_UNIT, SSHD_DROPIN};
use ferrum_platform::{Platform, ServiceAction, Sshd};
use std::path::Path;

pub const DROPIN: &str = "PasswordAuthentication no\nKbdInteractiveAuthentication no\n";

pub fn status(platform: &dyn Platform, sshd: Sshd) -> anyhow::Result<Ssh> {
    Ok(Ssh {
        port: sshd.port,
        password_auth: sshd.password_auth,
        keys: platform.authorized_keys()?,
    })
}

/// Refuses outright when no key is installed: a box without one is only reachable by password.
pub fn disable_passwords(platform: &dyn Platform) -> anyhow::Result<()> {
    if platform.authorized_keys()?.is_empty() {
        return Err(SecurityError::NoKeys.into());
    }
    let dropin = Path::new(SSHD_DROPIN);
    platform.write_file(dropin, DROPIN, 0o644)?;
    if let Err(e) = platform.sshd_test() {
        platform.remove_file(dropin)?;
        return Err(SecurityError::Host(e.to_string()).into());
    }
    platform.service(ServiceAction::ReloadOrRestart, SSH_UNIT)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrum_platform::FakePlatform;

    #[test]
    fn without_a_key_nothing_is_touched_and_with_one_the_dropin_is_tested_then_applied() {
        let p = FakePlatform::new();
        let refused = disable_passwords(&p).unwrap_err();
        assert!(matches!(
            refused.downcast_ref::<SecurityError>(),
            Some(SecurityError::NoKeys)
        ));
        assert!(refused.to_string().contains("/root/.ssh/authorized_keys"));
        assert!(p.calls_matching("write_file").is_empty());

        p.add_key("saeed@laptop");
        disable_passwords(&p).unwrap();
        let calls = p.calls();
        let wrote = calls
            .iter()
            .position(|c| c == "write_file /etc/ssh/sshd_config.d/10-ferrum.conf 644")
            .unwrap();
        let tested = calls.iter().position(|c| c == "sshd_test").unwrap();
        let reloaded = calls
            .iter()
            .position(|c| c == "service reload-or-restart ssh")
            .unwrap();
        assert!(wrote < tested && tested < reloaded, "{calls:#?}");
        assert_eq!(p.written(SSHD_DROPIN).as_deref(), Some(DROPIN));
        let s = status(&p, p.sshd_effective().unwrap()).unwrap();
        assert_eq!(s.keys.len(), 1);
        assert_eq!(s.port, 22);
    }

    #[test]
    fn a_dropin_sshd_rejects_is_removed_and_sshd_is_left_alone() {
        let p = FakePlatform::new();
        p.add_key("saeed@laptop");
        p.fail_next("sshd_test");
        let e = disable_passwords(&p).unwrap_err();
        assert!(matches!(
            e.downcast_ref::<SecurityError>(),
            Some(SecurityError::Host(_))
        ));
        assert!(p.removed(SSHD_DROPIN));
        assert!(p.calls_matching("service").is_empty());
    }
}
