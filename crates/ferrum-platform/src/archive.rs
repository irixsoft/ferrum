use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

pub fn extract_tar_gz(archive: &[u8], dest: &Path, strip_components: u32) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    let root = dest.canonicalize()?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    tar.set_preserve_permissions(false);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let Some(rel) = inside(&path, strip_components as usize) else {
            if path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err(escapes(&path));
            }
            continue;
        };
        let target = dest.join(rel);
        let parent = target.parent().unwrap_or(dest);
        if !nearest_existing(parent)?.starts_with(&root) {
            return Err(escapes(&path));
        }
        std::fs::create_dir_all(parent)?;
        if !parent.canonicalize()?.starts_with(&root) {
            return Err(escapes(&path));
        }
        let kind = entry.header().entry_type();
        if kind.is_hard_link() || kind.is_symlink() {
            let link = entry
                .link_name()?
                .ok_or_else(|| escapes(&path))?
                .into_owned();
            if kind.is_hard_link() {
                let source = inside(&link, strip_components as usize)
                    .map(|rel| dest.join(rel))
                    .ok_or_else(|| escapes(&path))?;
                if !source.canonicalize()?.starts_with(&root) {
                    return Err(escapes(&path));
                }
                std::fs::hard_link(&source, &target)?;
                continue;
            }
            if !resolve(parent, &link).starts_with(dest) {
                return Err(escapes(&path));
            }
        }
        entry.unpack(&target)?;
    }
    Ok(())
}

fn nearest_existing(path: &Path) -> io::Result<PathBuf> {
    let mut cur = path;
    loop {
        if let Ok(real) = cur.canonicalize() {
            return Ok(real);
        }
        cur = cur.parent().ok_or_else(|| escapes(path))?;
    }
}

fn resolve(dir: &Path, link: &Path) -> PathBuf {
    let mut out = dir.to_path_buf();
    for component in link.components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out = PathBuf::from(other.as_os_str()),
        }
    }
    out
}

pub fn extract_zip(archive: &[u8], dest: &Path, strip_components: u32) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut zip = zip::ZipArchive::new(io::Cursor::new(archive)).map_err(io::Error::other)?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(io::Error::other)?;
        let enclosed = file
            .enclosed_name()
            .ok_or_else(|| escapes(Path::new(file.name())))?;
        let Some(rel) = inside(&enclosed, strip_components as usize) else {
            continue;
        };
        let target = dest.join(rel);
        if file.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        io::copy(&mut file, &mut out)?;
        if let Some(mode) = file.unix_mode() {
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    Ok(())
}

fn inside(path: &Path, strip: usize) -> Option<PathBuf> {
    let mut rel = PathBuf::new();
    let mut kept = 0;
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                kept += 1;
                if kept > strip {
                    rel.push(part);
                }
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!rel.as_os_str().is_empty()).then_some(rel)
}

fn escapes(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "{} would be written outside the destination",
            path.display()
        ),
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn tar_gz(files: &[(&str, &[u8])]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        for (name, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(if name.contains("/bin/") { 0o755 } else { 0o644 });
            if name.contains("..") {
                header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name.as_bytes());
                header.set_cksum();
                tar.append(&header, *data).unwrap();
            } else {
                header.set_cksum();
                tar.append_data(&mut header, name, *data).unwrap();
            }
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    pub fn zip_of(files: &[(&str, &[u8], u32)]) -> Vec<u8> {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        for (name, data, mode) in files {
            let options = zip::write::SimpleFileOptions::default().unix_permissions(*mode);
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn a_node_style_tarball_is_unpacked_without_its_top_directory() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(&[
            ("node-v22.11.0-linux-x64/bin/node", b"#!node"),
            ("node-v22.11.0-linux-x64/LICENSE", b"MIT"),
        ]);
        extract_tar_gz(&archive, dir.path(), 1).unwrap();
        assert!(dir.path().join("bin/node").exists());
        assert!(dir.path().join("LICENSE").exists());
        assert!(!dir.path().join("node-v22.11.0-linux-x64").exists());
        let mode = std::fs::metadata(dir.path().join("bin/node"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the tarball's mode must be honoured");
    }

    #[test]
    fn a_bun_style_zip_is_unpacked_and_the_binary_is_executable() {
        let dir = tempfile::tempdir().unwrap();
        let archive = zip_of(&[("bun-linux-x64/bun", b"ELF", 0o755)]);
        extract_zip(&archive, dir.path(), 1).unwrap();
        let mode = std::fs::metadata(dir.path().join("bun"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the zip's unix mode must be honoured");
    }

    #[test]
    fn an_archive_that_escapes_its_destination_is_refused() {
        let outer = tempfile::tempdir().unwrap();
        let dest = outer.path().join("a/b");
        let archive = tar_gz(&[("../../etc/passwd", b"root::0:0")]);
        assert!(extract_tar_gz(&archive, &dest, 0).is_err());
        assert!(!outer.path().join("etc/passwd").exists());

        let archive = zip_of(&[("../evil", b"x", 0o644)]);
        assert!(extract_zip(&archive, &dest, 0).is_err());
        assert!(!outer.path().join("a/evil").exists());
    }

    type Tar = tar::Builder<flate2::write::GzEncoder<Vec<u8>>>;

    fn tar_with(build: impl FnOnce(&mut Tar)) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        build(&mut tar);
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn file(tar: &mut Tar, path: &str, data: &[u8], mode: u32) {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        tar.append_data(&mut header, path, data).unwrap();
    }

    fn link(tar: &mut Tar, kind: tar::EntryType, path: &str, target: &str) {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(kind);
        header.set_size(0);
        header.set_mode(0o777);
        tar.append_link(&mut header, path, target).unwrap();
    }

    #[test]
    fn a_link_that_leads_out_of_the_destination_is_refused_and_nothing_lands_outside() {
        let outer = tempfile::tempdir().unwrap();
        let dest = outer.path().join("dest");
        let outside = outer.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();

        let archive = tar_with(|t| {
            link(t, tar::EntryType::Symlink, "lib", "/etc");
            file(t, "lib/cron.d/evil", b"* * * * * root id", 0o644);
        });
        assert!(extract_tar_gz(&archive, &dest, 0).is_err());
        assert!(std::fs::symlink_metadata(dest.join("lib")).is_err());

        let archive = tar_with(|t| {
            link(t, tar::EntryType::Symlink, "up", "../outside");
            file(t, "up/evil", b"x", 0o644);
        });
        assert!(extract_tar_gz(&archive, &dest, 0).is_err());
        assert!(!outside.join("evil").exists());

        let archive = tar_with(|t| link(t, tar::EntryType::Link, "shadow", "/etc/passwd"));
        assert!(extract_tar_gz(&archive, &dest, 0).is_err());
        assert!(!dest.join("shadow").exists());

        std::os::unix::fs::symlink(&outside, dest.join("planted")).unwrap();
        let archive = tar_with(|t| file(t, "planted/a/b/evil", b"x", 0o644));
        assert!(extract_tar_gz(&archive, &dest, 0).is_err());
        assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    }

    #[test]
    fn setuid_and_setgid_bits_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_with(|t| file(t, "bin/tool", b"ELF", 0o4755));
        extract_tar_gz(&archive, dir.path(), 0).unwrap();
        let mode = std::fs::metadata(dir.path().join("bin/tool"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o7777, 0o755);
    }

    #[test]
    fn links_inside_the_destination_still_extract() {
        let dir = tempfile::tempdir().unwrap();
        let top = "node-v22.11.0-linux-x64";
        let archive = tar_with(|t| {
            file(
                t,
                &format!("{top}/lib/node_modules/npm/bin/npm-cli.js"),
                b"npm",
                0o755,
            );
            link(
                t,
                tar::EntryType::Symlink,
                &format!("{top}/bin/npm"),
                "../lib/node_modules/npm/bin/npm-cli.js",
            );
            link(
                t,
                tar::EntryType::Link,
                &format!("{top}/bin/npm-cli"),
                &format!("{top}/lib/node_modules/npm/bin/npm-cli.js"),
            );
        });
        extract_tar_gz(&archive, dir.path(), 1).unwrap();
        assert_eq!(std::fs::read(dir.path().join("bin/npm")).unwrap(), b"npm");
        assert_eq!(
            std::fs::read_link(dir.path().join("bin/npm")).unwrap(),
            Path::new("../lib/node_modules/npm/bin/npm-cli.js")
        );
        assert_eq!(
            std::fs::read(dir.path().join("bin/npm-cli")).unwrap(),
            b"npm"
        );
    }

    #[test]
    fn stripping_more_than_the_depth_skips_the_entry_rather_than_writing_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(&[("top/only", b"x")]);
        extract_tar_gz(&archive, dir.path(), 2).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
