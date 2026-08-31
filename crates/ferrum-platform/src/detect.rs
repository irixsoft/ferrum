use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    pub fn current() -> Option<Self> {
        match std::env::consts::ARCH {
            "x86_64" => Some(Self::X86_64),
            "aarch64" => Some(Self::Aarch64),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

#[derive(Debug, Clone)]
pub struct HostInfo {
    pub distro_id: String,
    pub version_id: String,
    pub arch: Arch,
    pub pretty_name: String,
}

pub const SUPPORTED_UBUNTU: [&str; 2] = ["22.04", "24.04"];

#[derive(Debug, thiserror::Error)]
pub struct Unsupported {
    found: String,
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Ferrum supports Ubuntu 22.04 and 24.04 on x86_64 and aarch64.\nThis host is {}.",
            self.found
        )
    }
}

pub fn parse_os_release(text: &str) -> Option<(String, String, String)> {
    let mut id = None;
    let mut version = None;
    let mut pretty = None;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').to_string();
        match k.trim() {
            "ID" => id = Some(v),
            "VERSION_ID" => version = Some(v),
            "PRETTY_NAME" => pretty = Some(v),
            _ => {}
        }
    }
    let id = id?;
    let version = version?;
    let pretty = pretty.unwrap_or_else(|| format!("{id} {version}"));
    Some((id, version, pretty))
}

pub fn check_supported(info: &HostInfo) -> Result<(), Unsupported> {
    if info.distro_id == "ubuntu" && SUPPORTED_UBUNTU.contains(&info.version_id.as_str()) {
        return Ok(());
    }
    Err(Unsupported {
        found: format!("{} ({})", info.pretty_name, info.arch.as_str()),
    })
}

pub fn detect() -> Result<HostInfo, Unsupported> {
    let arch = Arch::current().ok_or_else(|| Unsupported {
        found: std::env::consts::ARCH.into(),
    })?;
    let text = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let (distro_id, version_id, pretty_name) = parse_os_release(&text).unwrap_or_else(|| {
        (
            "unknown".into(),
            "unknown".into(),
            "an unrecognised system".into(),
        )
    });
    let info = HostInfo {
        distro_id,
        version_id,
        arch,
        pretty_name,
    };
    check_supported(&info)?;
    Ok(info)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOBLE: &str = r#"PRETTY_NAME="Ubuntu 24.04.1 LTS"
NAME="Ubuntu"
VERSION_ID="24.04"
ID=ubuntu
ID_LIKE=debian
"#;

    const DEBIAN: &str = r#"PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
NAME="Debian GNU/Linux"
VERSION_ID="12"
ID=debian
"#;

    #[test]
    fn parses_quoted_and_unquoted_fields() {
        let (id, version, pretty) = parse_os_release(NOBLE).unwrap();
        assert_eq!(id, "ubuntu");
        assert_eq!(version, "24.04");
        assert_eq!(pretty, "Ubuntu 24.04.1 LTS");
    }

    #[test]
    fn accepts_both_supported_ubuntu_releases() {
        for v in ["22.04", "24.04"] {
            let info = HostInfo {
                distro_id: "ubuntu".into(),
                version_id: v.into(),
                arch: Arch::X86_64,
                pretty_name: format!("Ubuntu {v}"),
            };
            assert!(check_supported(&info).is_ok(), "{v} should be supported");
        }
    }

    #[test]
    fn refuses_other_distros_by_name() {
        let (id, version, pretty) = parse_os_release(DEBIAN).unwrap();
        let info = HostInfo {
            distro_id: id,
            version_id: version,
            arch: Arch::X86_64,
            pretty_name: pretty,
        };
        let err = check_supported(&info).unwrap_err().to_string();
        assert!(err.contains("Debian GNU/Linux 12"), "got: {err}");
        assert!(err.contains("Ubuntu 22.04"), "got: {err}");
    }

    #[test]
    fn refuses_unsupported_ubuntu_release() {
        let info = HostInfo {
            distro_id: "ubuntu".into(),
            version_id: "20.04".into(),
            arch: Arch::X86_64,
            pretty_name: "Ubuntu 20.04 LTS".into(),
        };
        assert!(check_supported(&info).is_err());
    }

    #[test]
    fn resolve_package_is_identity_on_ubuntu() {
        use crate::Platform;
        let p = crate::ubuntu::Ubuntu;
        assert_eq!(p.resolve_package("ffmpeg"), vec!["ffmpeg".to_string()]);
        assert_eq!(
            p.resolve_package("poppler-utils"),
            vec!["poppler-utils".to_string()]
        );
    }
}
