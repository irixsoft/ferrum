use ferrum_platform::ubuntu::{REDIS_DISTRO_UNIT, REDIS_KEY_URL, REDIS_SERVER, redis_repo_line};
use ferrum_platform::{Platform, ServiceAction};
use std::path::Path;

const REPO_NAME: &str = "redis";
const PACKAGE: &str = "redis";

pub fn installed(platform: &dyn Platform) -> bool {
    platform.file_exists(Path::new(REDIS_SERVER))
}

/// The distro unit is masked before the package lands, so its postinst never starts a default
/// instance on 6379.
pub fn ensure_installed(platform: &dyn Platform, codename: &str) -> anyhow::Result<()> {
    if installed(platform) {
        return Ok(());
    }
    platform.add_apt_repo(REPO_NAME, REDIS_KEY_URL, &redis_repo_line(codename))?;
    platform.service(ServiceAction::Mask, REDIS_DISTRO_UNIT)?;
    platform.install_packages(&[PACKAGE])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrum_platform::FakePlatform;

    #[test]
    fn the_distro_redis_is_masked_before_it_can_take_6379() {
        let p = FakePlatform::new();
        ensure_installed(&p, "noble").unwrap();
        let calls = p.calls();
        let repo = calls
            .iter()
            .position(|c| {
                c == "add_apt_repo redis https://packages.redis.io/gpg https://packages.redis.io/deb noble main"
            })
            .unwrap();
        let mask = calls
            .iter()
            .position(|c| c == "service mask redis-server")
            .unwrap();
        let pkg = calls
            .iter()
            .position(|c| c == "install_packages redis")
            .unwrap();
        assert!(repo < mask && mask < pkg, "{calls:#?}");
    }

    #[test]
    fn an_installed_redis_is_left_alone() {
        let p = FakePlatform::new();
        p.write_file(Path::new(REDIS_SERVER), "", 0o755).unwrap();
        ensure_installed(&p, "noble").unwrap();
        assert!(p.calls_matching("install_packages").is_empty());
        assert!(p.calls_matching("add_apt_repo").is_empty());
    }
}
