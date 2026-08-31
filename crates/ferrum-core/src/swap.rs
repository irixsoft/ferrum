use ferrum_platform::{Platform, PlatformError};
use std::path::Path;

pub const SWAP_PATH: &str = "/swapfile";

pub fn recommended_mb(total_memory_kb: u64) -> u64 {
    let mb = total_memory_kb / 1024;
    (mb * 2).clamp(1024, 4096)
}

pub fn needs_swap(platform: &dyn Platform) -> Result<bool, PlatformError> {
    Ok(platform.swap_total_kb()? == 0)
}

pub fn create(platform: &dyn Platform, size_mb: u64) -> Result<(), PlatformError> {
    platform.create_swapfile(Path::new(SWAP_PATH), size_mb)?;
    platform.set_sysctl("vm.swappiness", "10")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrum_platform::FakePlatform;

    #[test]
    fn sizing_is_twice_ram_capped_at_4gb() {
        assert_eq!(recommended_mb(1_048_576), 2048);
        assert_eq!(recommended_mb(2_097_152), 4096);
        assert_eq!(recommended_mb(8_388_608), 4096);
    }

    #[test]
    fn tiny_hosts_still_get_a_useful_floor() {
        assert_eq!(recommended_mb(524_288), 1024);
    }

    #[test]
    fn needs_swap_only_when_there_is_none() {
        let p = FakePlatform::new();
        p.set_swap_kb(0);
        assert!(needs_swap(&p).unwrap());
        p.set_swap_kb(2_097_152);
        assert!(!needs_swap(&p).unwrap());
    }

    #[test]
    fn create_sets_swappiness_low() {
        let p = FakePlatform::new();
        create(&p, 2048).unwrap();
        let calls = p.calls();
        assert!(
            calls
                .iter()
                .any(|c| c.starts_with("create_swapfile /swapfile 2048")),
            "{calls:?}"
        );
        assert!(
            calls.iter().any(|c| c == "set_sysctl vm.swappiness 10"),
            "{calls:?}"
        );
    }
}
