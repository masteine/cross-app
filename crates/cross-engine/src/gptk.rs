//! Importing Apple's Game Porting Toolkit (D3DMetal).
//!
//! The toolkit cannot be redistributed, so the user downloads
//! `Game_Porting_Toolkit_*.dmg` from developer.apple.com and points us at it.
//! We mount it read-only, copy `lib/external/` into our own data directory and
//! record the version. Accepting Apple's license stays the user's job: if the
//! image shows one, mount it once in Finder and import from the mounted volume.
//!
//! Besides `lib/external/` (D3DMetal.framework + `libd3dshared.dylib`) the
//! toolkit's `lib/wine/x86_64-windows/` holds Apple's PE forwarders
//! (`d3d10.dll`, `d3d11.dll`, `d3d12.dll`, `dxgi.dll`, ...). They are copied too,
//! and the unix-side names (`d3d11.so` ...) are recreated as symlinks to
//! `../../external/libd3dshared.dylib`, which is how Apple's payload is laid out.
//! Sources for that layout: frankea/Whisky `WhiskyKit/.../GPTKImporter.swift`
//! (`forwarderDLLNames`, `unixLibraryNames`, `unixLinkDestination`) and the
//! deployment report in https://github.com/frankea/Whisky/issues/163.
//! Deploying it into an engine is [`crate::d3dmetal`]'s job.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, Result};

/// Directory (relative to the GPTK dir) that holds D3DMetal and friends.
pub const LIB_EXTERNAL: &str = "lib/external";
/// File recording which toolkit version was imported.
pub const VERSION_FILE: &str = "VERSION";
/// Directory (relative to the GPTK dir) with Apple's PE forwarders.
pub const LIB_WINE_WINDOWS: &str = "lib/wine/x86_64-windows";
/// Directory (relative to the GPTK dir) with the unix-side bridge names.
pub const LIB_WINE_UNIX: &str = "lib/wine/x86_64-unix";
/// The shared library every unix bridge name resolves to.
pub const SHARED_LIB: &str = "libd3dshared.dylib";
/// Target of every unix bridge symlink, relative to `x86_64-unix/`.
pub const UNIX_LINK_TARGET: &str = "../../external/libd3dshared.dylib";
/// The D3D forwarders deployed into an engine. GPTK 4 has no d3d9 forwarder.
/// Apple also ships `nvapi64.dll` and `nvngx-on-metalfx.dll`; they are copied
/// on import but not deployed (see `docs/graphics.md`).
pub const FORWARDERS: [&str; 4] = ["d3d10.dll", "d3d11.dll", "d3d12.dll", "dxgi.dll"];
/// Wine marks the PE files it builds with this string right after the DOS
/// header (offset 0x40); see `builtin_signature` in Wine's
/// `dlls/ntdll/unix/loader.c`. Apple's forwarders carry it.
const BUILTIN_SIGNATURE: &[u8] = b"Wine builtin DLL";
const BUILTIN_SIGNATURE_OFFSET: usize = 0x40;

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
    if let Some(lib) = external.parent() {
        copy_forwarders(&lib.join("wine/x86_64-windows"), staging)?;
    }
    let version_file = staging.join(VERSION_FILE);
    let text = match version {
        Some(v) => format!("{v}\n"),
        None => String::new(),
    };
    fs::write(&version_file, text).map_err(|e| Error::io(version_file, e))
}

/// Copy every `*.dll` of Apple's `x86_64-windows` folder (if the toolkit has
/// one) and recreate the unix bridge names for the forwarders found.
fn copy_forwarders(windows: &Path, staging: &Path) -> Result<()> {
    let Ok(entries) = fs::read_dir(windows) else {
        return Ok(()); // an older toolkit without forwarders: external only
    };
    let target = staging.join(LIB_WINE_WINDOWS);
    fs::create_dir_all(&target).map_err(|e| Error::io(&target, e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dll = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dll"));
        if is_dll && path.is_file() {
            let to = target.join(entry.file_name());
            fs::copy(&path, &to).map_err(|e| Error::io(&to, e))?;
        }
    }
    for name in FORWARDERS {
        let dll = target.join(name);
        if dll.is_file() && !is_builtin_pe(&dll)? {
            return Err(Error::NotBuiltinDll { path: dll });
        }
    }
    link_unix_bridges(&staging.join(LIB_WINE_UNIX), &target)
}

/// `<x86_64-unix>/<name>.so -> ../../external/libd3dshared.dylib` for every
/// forwarder present in `windows`. Existing entries are replaced.
pub fn link_unix_bridges(unix: &Path, windows: &Path) -> Result<()> {
    fs::create_dir_all(unix).map_err(|e| Error::io(unix, e))?;
    for name in FORWARDERS {
        if !windows.join(name).is_file() {
            continue;
        }
        let link = unix.join(name.replace(".dll", ".so"));
        if link.symlink_metadata().is_ok() {
            fs::remove_file(&link).map_err(|e| Error::io(&link, e))?;
        }
        std::os::unix::fs::symlink(UNIX_LINK_TARGET, &link).map_err(|e| Error::io(&link, e))?;
    }
    Ok(())
}

/// Whether `path` is a PE file built by winebuild (Apple's forwarders are).
pub fn is_builtin_pe(path: &Path) -> Result<bool> {
    use std::io::Read;
    let mut head = [0u8; BUILTIN_SIGNATURE_OFFSET + 16];
    let mut file = fs::File::open(path).map_err(|e| Error::io(path, e))?;
    let mut read = 0;
    while read < head.len() {
        match file.read(&mut head[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(e) => return Err(Error::io(path, e)),
        }
    }
    Ok(read == head.len()
        && head.starts_with(b"MZ")
        && &head[BUILTIN_SIGNATURE_OFFSET..] == BUILTIN_SIGNATURE)
}

/// What an import under `dir` contains, for deciding whether D3DMetal can be
/// deployed from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    pub dir: PathBuf,
    pub version: Option<String>,
    /// Required files that are not there, relative to `dir`.
    pub missing: Vec<String>,
}

impl Payload {
    pub fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }

    pub fn windows_dir(&self) -> PathBuf {
        self.dir.join(LIB_WINE_WINDOWS)
    }

    pub fn external_dir(&self) -> PathBuf {
        self.dir.join(LIB_EXTERNAL)
    }

    /// Error unless every file D3DMetal deployment needs is present.
    pub fn require_complete(&self) -> Result<()> {
        if self.is_complete() {
            Ok(())
        } else {
            Err(Error::GptkIncomplete {
                dir: self.dir.clone(),
                missing: self.missing.clone(),
            })
        }
    }
}

/// Inspect an import made by [`import`]; `None` when there is none at `dir`.
/// Imports made before the forwarders were copied report them as missing.
pub fn inspect(dir: &Path) -> Option<Payload> {
    if !dir.join(LIB_EXTERNAL).is_dir() {
        return None;
    }
    let version = fs::read_to_string(dir.join(VERSION_FILE))
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty());
    let mut missing = Vec::new();
    for path in [
        format!("{LIB_EXTERNAL}/{SHARED_LIB}"),
        format!("{LIB_EXTERNAL}/D3DMetal.framework"),
    ] {
        if !dir.join(&path).exists() {
            missing.push(path);
        }
    }
    for name in FORWARDERS {
        let path = format!("{LIB_WINE_WINDOWS}/{name}");
        if !dir.join(&path).is_file() {
            missing.push(path);
        }
    }
    Some(Payload {
        dir: dir.to_owned(),
        version,
        missing,
    })
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
pub(crate) mod tests {
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

    /// A synthetic GPTK tree with builtin-marked forwarders (not Apple's code).
    pub(crate) fn fake_full_toolkit(root: &Path) {
        fake_toolkit(root);
        let windows = root.join("redist/lib/wine/x86_64-windows");
        fs::create_dir_all(&windows).unwrap();
        for name in FORWARDERS.iter().chain(&["nvngx-on-metalfx.dll"]) {
            fs::write(windows.join(name), fake_builtin_dll(name)).unwrap();
        }
        // Apple's unix side: symlinks; ours are ignored and recreated on import.
        fs::create_dir_all(root.join("redist/lib/wine/x86_64-unix")).unwrap();
    }

    pub(crate) fn fake_builtin_dll(tag: &str) -> Vec<u8> {
        let mut bytes = vec![0u8; BUILTIN_SIGNATURE_OFFSET];
        bytes[..2].copy_from_slice(b"MZ");
        bytes.extend_from_slice(BUILTIN_SIGNATURE);
        bytes.extend_from_slice(tag.as_bytes());
        bytes
    }

    #[test]
    fn imports_forwarders_and_links_unix_bridges() {
        let tmp = tempfile::tempdir().unwrap();
        let mounted = tmp.path().join("Game_Porting_Toolkit_4.0");
        fake_full_toolkit(&mounted);
        let dest = tmp.path().join("gptk");
        import(&mounted, &dest, None).unwrap();

        let payload = inspect(&dest).unwrap();
        assert!(payload.is_complete(), "{:?}", payload.missing);
        assert_eq!(payload.version.as_deref(), Some("4.0"));
        assert!(dest
            .join(LIB_WINE_WINDOWS)
            .join("nvngx-on-metalfx.dll")
            .is_file());
        let link = dest.join(LIB_WINE_UNIX).join("d3d12.so");
        assert_eq!(fs::read_link(&link).unwrap(), Path::new(UNIX_LINK_TARGET));
        // The link resolves to the shared library inside the import.
        assert_eq!(fs::read_to_string(&link).unwrap(), "lib");
    }

    #[test]
    fn old_imports_report_missing_forwarders() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(inspect(tmp.path()).is_none());
        let mounted = tmp.path().join("gptk3");
        fake_toolkit(&mounted);
        let dest = tmp.path().join("gptk");
        import(&mounted, &dest, Some("3.0")).unwrap();
        let payload = inspect(&dest).unwrap();
        assert!(!payload.is_complete());
        assert!(payload
            .missing
            .contains(&"lib/wine/x86_64-windows/dxgi.dll".to_owned()));
        let err = payload.require_complete().unwrap_err().to_string();
        assert!(err.contains("re-import"), "{err}");
    }

    #[test]
    fn rejects_forwarders_that_are_not_wine_builtins() {
        let tmp = tempfile::tempdir().unwrap();
        let mounted = tmp.path().join("fake");
        fake_full_toolkit(&mounted);
        fs::write(
            mounted.join("redist/lib/wine/x86_64-windows/d3d11.dll"),
            b"MZ not a builtin",
        )
        .unwrap();
        let err = import(&mounted, &tmp.path().join("gptk"), None).unwrap_err();
        assert!(matches!(err, Error::NotBuiltinDll { .. }), "{err}");
        assert!(!tmp.path().join("gptk").exists());
    }

    #[test]
    fn builtin_marker_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("x.dll");
        fs::write(&p, fake_builtin_dll("x")).unwrap();
        assert!(is_builtin_pe(&p).unwrap());
        fs::write(&p, b"MZ").unwrap();
        assert!(!is_builtin_pe(&p).unwrap());
        let mut native = fake_builtin_dll("x");
        native[0x40] = b'X';
        fs::write(&p, native).unwrap();
        assert!(!is_builtin_pe(&p).unwrap());
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
