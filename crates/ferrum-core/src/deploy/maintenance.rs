use crate::PAGES_DIR;
use crate::apps::provision::app_dir;
use ferrum_platform::{Platform, PlatformError};
use std::path::{Path, PathBuf};

const PAGE: &str = include_str!("../../../../packaging/maintenance.html");
pub const PAGE_NAME: &str = "maintenance.html";

pub fn page_path() -> PathBuf {
    Path::new(PAGES_DIR).join(PAGE_NAME)
}

pub fn flag_path(slug: &str) -> PathBuf {
    app_dir(slug).join("maintenance")
}

/// nginx serves the page itself, so it only has to exist; no reload is involved.
pub fn ensure_page(platform: &dyn Platform) -> Result<(), PlatformError> {
    let path = page_path();
    if platform.file_exists(&path) {
        return Ok(());
    }
    platform.make_dirs(Path::new(PAGES_DIR), 0o755)?;
    platform.write_file(&path, PAGE, 0o644)
}

pub fn on(platform: &dyn Platform, slug: &str) -> Result<(), PlatformError> {
    ensure_page(platform)?;
    platform.write_file(&flag_path(slug), "", 0o644)
}

pub fn off(platform: &dyn Platform, slug: &str) -> Result<(), PlatformError> {
    platform.remove_file(&flag_path(slug))
}

pub fn is_on(platform: &dyn Platform, slug: &str) -> bool {
    platform.file_exists(&flag_path(slug))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferrum_platform::FakePlatform;

    #[test]
    fn the_flag_toggles_without_a_reload_and_the_page_is_written_once() {
        let p = FakePlatform::new();
        on(&p, "ledger").unwrap();
        assert!(is_on(&p, "ledger"));
        assert!(
            p.written("/var/lib/ferrum/pages/maintenance.html")
                .unwrap()
                .contains("<html")
        );
        off(&p, "ledger").unwrap();
        assert!(!is_on(&p, "ledger"));
        on(&p, "ledger").unwrap();
        assert_eq!(
            p.calls_matching("write_file /var/lib/ferrum/pages").len(),
            1
        );
        assert!(!p.calls().iter().any(|c| c.starts_with("service")));
        assert_eq!(
            flag_path("ledger"),
            Path::new("/var/lib/ferrum/apps/ledger/maintenance")
        );
    }
}
