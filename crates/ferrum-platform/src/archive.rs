use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

pub fn extract_tar_gz(archive: &[u8], dest: &Path, strip_components: u32) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    tar.set_preserve_permissions(true);
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
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        entry.unpack(&target)?;
    }
    Ok(())
}

pub fn extract_zip(archive: &[u8], dest: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut zip = zip::ZipArchive::new(io::Cursor::new(archive)).map_err(io::Error::other)?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(io::Error::other)?;
        let rel = file
            .enclosed_name()
            .ok_or_else(|| escapes(Path::new(file.name())))?;
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
        extract_zip(&archive, dir.path()).unwrap();
        let mode = std::fs::metadata(dir.path().join("bun-linux-x64/bun"))
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
        assert!(extract_zip(&archive, &dest).is_err());
        assert!(!outer.path().join("a/evil").exists());
    }

    #[test]
    fn stripping_more_than_the_depth_skips_the_entry_rather_than_writing_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let archive = tar_gz(&[("top/only", b"x")]);
        extract_tar_gz(&archive, dir.path(), 2).unwrap();
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }
}
