//! Importing Apple's Game Porting Toolkit (D3DMetal).
//!
//! The toolkit cannot be redistributed, so the user downloads
//! `Game_Porting_Toolkit_*.dmg` from developer.apple.com and points us at it.
//! We mount it read-only, copy `lib/external/` into our own data directory and
//! record the version. Accepting Apple's license stays the user's job: if the
//! image shows one, mount it once in Finder and import from the mounted volume.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, Result};

/// Directory (relative to the GPTK dir) that holds D3DMetal and friends.
pub const LIB_EXTERNAL: &str = "lib/external";
/// File recording which toolkit version was imported.
pub const VERSION_FILE: &str = "VERSION";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GptkImport {
    pub dir: PathBuf,
    pub version: Option<String>,
}

/// Import GPTK from a `.dmg` or from an already-mounted folder into `dest`,
/// replacing any previous import. `version` overrides detection from the file name.
pub fn import(source: &Path, dest: &Path, version: Option<&str>) -> Result<GptkImport> {
    let version = version
        .map(str::to_owned)
        .or_else(|| detect_version(source));

    let is_dmg = source
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("dmg"));
    if is_dmg {
        let mount = Mount::attach(source)?;
        import_from_dir(mount.path(), dest, version)
    } else {
        import_from_dir(source, dest, version)
    }
}

fn import_from_dir(root: &Path, dest: &Path, version: Option<String>) -> Result<GptkImport> {
    let external = find_lib_external(root).ok_or_else(|| Error::GptkNotFound(root.to_owned()))?;

    let parent = dest.parent().unwrap_or(dest);
    fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    let staging = parent.join(".gptk.importing");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|e| Error::io(&staging, e))?;
    }

    let result = copy_into(&external, &staging, version.as_deref());
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    if dest.exists() {
        fs::remove_dir_all(dest).map_err(|e| Error::io(dest, e))?;
    }
    fs::rename(&staging, dest).map_err(|e| Error::io(dest, e))?;
    tracing::info!(dir = %dest.display(), ?version, "GPTK imported");
    Ok(GptkImport {
        dir: dest.to_owned(),
        version,
    })
}

fn copy_into(external: &Path, staging: &Path, version: Option<&str>) -> Result<()> {
    let target = staging.join(LIB_EXTERNAL);
    fs::create_dir_all(&target).map_err(|e| Error::io(&target, e))?;
    // `ditto` preserves symlinks and framework layout, unlike a naive recursive copy.
    run("ditto", &[external.as_os_str(), target.as_os_str()])?;
    let version_file = staging.join(VERSION_FILE);
    let text = match version {
        Some(v) => format!("{v}\n"),
        None => String::new(),
    };
    fs::write(&version_file, text).map_err(|e| Error::io(version_file, e))
}

/// Find a `lib/external` directory at most four levels below `root`
/// (breadth-first, so the shallowest wins).
pub fn find_lib_external(root: &Path) -> Option<PathBuf> {
    let mut level = vec![root.to_owned()];
    for _ in 0..=4 {
        let mut next = Vec::new();
        for dir in &level {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                    continue;
                }
                let is_external = path.file_name().is_some_and(|n| n == "external")
                    && dir.file_name().is_some_and(|n| n == "lib");
                if is_external {
                    return Some(path);
                }
                next.push(path);
            }
        }
        level = next;
    }
    None
}

/// First dotted number in the file or folder name:
/// `Game_Porting_Toolkit_3.0.dmg` -> `3.0`.
pub fn detect_version(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    // Not `file_stem`: for a mounted folder like `Toolkit 3.0` it would eat `.0`.
    let stem = match name.len().checked_sub(4) {
        Some(at) if name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(".dmg") => {
            &name[..at]
        }
        _ => name,
    };
    let bytes = stem.as_bytes();
    let start = bytes.iter().position(u8::is_ascii_digit)?;
    let mut end = start;
    while end < bytes.len()
        && (bytes[end].is_ascii_digit()
            || (bytes[end] == b'.' && bytes.get(end + 1).is_some_and(u8::is_ascii_digit)))
    {
        end += 1;
    }
    Some(stem[start..end].to_owned())
}

/// A mounted disk image, detached on drop.
struct Mount {
    point: PathBuf,
}

impl Mount {
    fn attach(dmg: &Path) -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let point = std::env::temp_dir().join(format!("cross-gptk-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&point).map_err(|e| Error::io(&point, e))?;
        let attach = run(
            "hdiutil",
            &[
                "attach".as_ref(),
                "-nobrowse".as_ref(),
                "-readonly".as_ref(),
                "-noverify".as_ref(),
                "-mountpoint".as_ref(),
                point.as_os_str(),
                dmg.as_os_str(),
            ],
        );
        if let Err(e) = attach {
            let _ = fs::remove_dir(&point);
            return Err(e);
        }
        Ok(Self { point })
    }

    fn path(&self) -> &Path {
        &self.point
    }
}

impl Drop for Mount {
    fn drop(&mut self) {
        let detached = run("hdiutil", &["detach".as_ref(), self.point.as_os_str()]).is_ok()
            || run(
                "hdiutil",
                &["detach".as_ref(), "-force".as_ref(), self.point.as_os_str()],
            )
            .is_ok();
        if detached {
            let _ = fs::remove_dir(&self.point);
        } else {
            tracing::warn!(point = %self.point.display(), "could not detach the GPTK disk image");
        }
    }
}

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| Error::Command {
            program: program.to_owned(),
            message: e.to_string(),
        })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut message = stderr.trim().to_owned();
    if message.is_empty() {
        message = format!("exited with {}", output.status);
    }
    if program == "hdiutil" && message.to_lowercase().contains("license") {
        message.push_str(
            "\nhint: mount the image once in Finder, accept the license, \
             then run the import on the mounted volume (e.g. /Volumes/...)",
        );
    }
    if program == "hdiutil" && message.contains("Resource temporarily unavailable") {
        message.push_str(
            "\nhint: the image may already be attached or mounted (e.g. opened in Finder); \
             run the import on the mounted volume (e.g. /Volumes/...) or eject it first",
        );
    }
    Err(Error::Command {
        program: program.to_owned(),
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_versions_from_names() {
        let v = |s: &str| detect_version(Path::new(s));
        assert_eq!(v("Game_Porting_Toolkit_3.0.dmg").as_deref(), Some("3.0"));
        assert_eq!(
            v("/x/Game Porting Toolkit 2.1.1.dmg").as_deref(),
            Some("2.1.1")
        );
        assert_eq!(v("GPTK-4.dmg").as_deref(), Some("4"));
        assert_eq!(v("toolkit.dmg"), None);
        assert_eq!(
            v("/Volumes/Game Porting Toolkit 3.0").as_deref(),
            Some("3.0")
        );
        assert_eq!(v("GPTK_3.0.DMG").as_deref(), Some("3.0"));
    }

    fn fake_toolkit(root: &Path) {
        let external = root.join("redist/lib/external");
        fs::create_dir_all(external.join("D3DMetal.framework")).unwrap();
        fs::write(external.join("D3DMetal.framework/D3DMetal"), "bin").unwrap();
        fs::write(external.join("libd3dshared.dylib"), "lib").unwrap();
    }

    #[test]
    fn finds_lib_external_in_nested_layout() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(find_lib_external(tmp.path()), None);
        fake_toolkit(tmp.path());
        assert_eq!(
            find_lib_external(tmp.path()).unwrap(),
            tmp.path().join("redist/lib/external")
        );
    }

    #[test]
    fn imports_from_a_mounted_folder_and_replaces_old_import() {
        let tmp = tempfile::tempdir().unwrap();
        let mounted = tmp.path().join("Game_Porting_Toolkit_3.0");
        fake_toolkit(&mounted);
        let dest = tmp.path().join("home/gptk");

        let imported = import(&mounted, &dest, None).unwrap();
        assert_eq!(imported.version.as_deref(), Some("3.0"));
        assert!(dest
            .join("lib/external/D3DMetal.framework/D3DMetal")
            .is_file());
        assert_eq!(
            fs::read_to_string(dest.join(VERSION_FILE)).unwrap(),
            "3.0\n"
        );

        // Re-import with an explicit version replaces the old tree.
        fs::write(dest.join("stale"), "").unwrap();
        import(&mounted, &dest, Some("3.1")).unwrap();
        assert!(!dest.join("stale").exists());
        assert_eq!(
            fs::read_to_string(dest.join(VERSION_FILE)).unwrap(),
            "3.1\n"
        );
        assert!(!tmp.path().join("home/.gptk.importing").exists());
    }

    #[test]
    fn rejects_folders_without_lib_external() {
        let tmp = tempfile::tempdir().unwrap();
        let err = import(tmp.path(), &tmp.path().join("out/gptk"), None).unwrap_err();
        assert!(matches!(err, Error::GptkNotFound(_)), "{err}");
        assert!(!tmp.path().join("out/gptk").exists());
    }

    /// Exercises the real `hdiutil` mount/detach path with a synthetic image.
    #[cfg(target_os = "macos")]
    #[test]
    fn imports_from_a_dmg() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        fake_toolkit(&src);
        let dmg = tmp.path().join("Game_Porting_Toolkit_9.9.dmg");
        let made = Command::new("hdiutil")
            .args(["create", "-quiet", "-srcfolder"])
            .arg(&src)
            .arg("-format")
            .arg("UDZO")
            .arg(&dmg)
            .status()
            .unwrap();
        assert!(made.success(), "hdiutil create failed");

        let dest = tmp.path().join("gptk");
        let imported = import(&dmg, &dest, None).unwrap();
        assert_eq!(imported.version.as_deref(), Some("9.9"));
        assert!(dest.join("lib/external/libd3dshared.dylib").is_file());
        // No volume left mounted from our temp mountpoint.
        let mounts = Command::new("mount").output().unwrap();
        assert!(!String::from_utf8_lossy(&mounts.stdout).contains("cross-gptk-"));
    }
}
