use super::Updates;
use ferrum_platform::Platform;
use ferrum_platform::ubuntu::APT_AUTO_UPGRADES;
use std::path::Path;

const PACKAGE: &str = "unattended-upgrades";
pub const AUTO_UPGRADES: &str =
    "APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"1\";\n";
const ENABLED_LINE: &str = "APT::Periodic::Unattended-Upgrade \"1\";";

pub fn status(platform: &dyn Platform) -> anyhow::Result<Updates> {
    let text = platform
        .read_file(Path::new(APT_AUTO_UPGRADES))?
        .unwrap_or_default();
    Ok(Updates {
        enabled: text.lines().any(|l| l.trim() == ENABLED_LINE),
    })
}

/// Ubuntu's `50unattended-upgrades` already allows `-security`; this turns the timer on.
pub fn enable(platform: &dyn Platform) -> anyhow::Result<()> {
    platform.install_packages(&[PACKAGE])?;
    platform.write_file(Path::new(APT_AUTO_UPGRADES), AUTO_UPGRADES, 0o644)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrum_platform::FakePlatform;

    #[test]
    fn enabling_installs_the_package_and_switches_the_periodic_upgrade_on() {
        let p = FakePlatform::new();
        assert!(!status(&p).unwrap().enabled);
        p.write_file(
            Path::new(APT_AUTO_UPGRADES),
            "APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"0\";\n",
            0o644,
        )
        .unwrap();
        assert!(!status(&p).unwrap().enabled);
        enable(&p).unwrap();
        assert!(status(&p).unwrap().enabled);
        assert_eq!(
            p.calls_matching("install_packages"),
            vec!["install_packages unattended-upgrades"]
        );
        assert_eq!(p.written(APT_AUTO_UPGRADES).as_deref(), Some(AUTO_UPGRADES));
    }
}
