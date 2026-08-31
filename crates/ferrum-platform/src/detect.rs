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
    pub codename: String,
    pub arch: Arch,
    pub pretty_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsRelease {
    pub id: String,
    pub version_id: String,
    pub codename: String,
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

pub fn parse_os_release(text: &str) -> Option<OsRelease> {
    let mut id = None;
    let mut version = None;
    let mut codename = None;
    let mut pretty = None;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').to_string();
        match k.trim() {
            "ID" => id = Some(v),
            "VERSION_ID" => version = Some(v),
            "VERSION_CODENAME" => codename = Some(v),
            "PRETTY_NAME" => pretty = Some(v),
            _ => {}
        }
    }
    let id = id?;
    let version_id = version?;
    let pretty_name = pretty.unwrap_or_else(|| format!("{id} {version_id}"));
    Some(OsRelease {
        id,
        version_id,
        codename: codename.unwrap_or_default(),
        pretty_name,
    })
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
    let os = parse_os_release(&text).unwrap_or_else(|| OsRelease {
        id: "unknown".into(),
        version_id: "unknown".into(),
        codename: String::new(),
        pretty_name: "an unrecognised system".into(),
    });
    let info = HostInfo {
        distro_id: os.id,
        version_id: os.version_id,
        codename: os.codename,
        arch,
        pretty_name: os.pretty_name,
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
VERSION_CODENAME=noble
ID=ubuntu
ID_LIKE=debian
"#;

    const JAMMY: &str = r#"PRETTY_NAME="Ubuntu 22.04.5 LTS"
NAME="Ubuntu"
VERSION_ID="22.04"
VERSION_CODENAME=jammy
ID=ubuntu
"#;

    const DEBIAN: &str = r#"PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
NAME="Debian GNU/Linux"
VERSION_ID="12"
ID=debian
"#;

    #[test]
    fn parses_quoted_and_unquoted_fields() {
        let os = parse_os_release(NOBLE).unwrap();
        assert_eq!(os.id, "ubuntu");
        assert_eq!(os.version_id, "24.04");
        assert_eq!(os.pretty_name, "Ubuntu 24.04.1 LTS");
    }

    #[test]
    fn reads_the_codename_for_both_supported_releases() {
        assert_eq!(parse_os_release(NOBLE).unwrap().codename, "noble");
        assert_eq!(parse_os_release(JAMMY).unwrap().codename, "jammy");
    }

    #[test]
    fn accepts_both_supported_ubuntu_releases() {
        for v in ["22.04", "24.04"] {
            let info = HostInfo {
                distro_id: "ubuntu".into(),
                version_id: v.into(),
                codename: "noble".into(),
                arch: Arch::X86_64,
                pretty_name: format!("Ubuntu {v}"),
            };
            assert!(check_supported(&info).is_ok(), "{v} should be supported");
        }
    }

    #[test]
    fn refuses_other_distros_by_name() {
        let os = parse_os_release(DEBIAN).unwrap();
        let info = HostInfo {
            distro_id: os.id,
            version_id: os.version_id,
            codename: os.codename,
            arch: Arch::X86_64,
            pretty_name: os.pretty_name,
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
            codename: "focal".into(),
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
