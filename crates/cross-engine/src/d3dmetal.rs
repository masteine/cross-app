//! Plugging Apple's D3DMetal into an engine.
//!
//! How D3DMetal reaches a Wine build (Apple's README for GPTK-ready Wine builds,
//! as followed by frankea/Whisky `GPTKImporter+Deployment.swift` and reported in
//! https://github.com/frankea/Whisky/issues/163):
//!
//! 1. `lib/external/` (D3DMetal.framework + `libd3dshared.dylib`) goes next to
//!    Wine's own libraries, at `<wine>/lib/external/`;
//! 2. Apple's PE forwarders replace Wine's builtin `d3d10`, `d3d11`, `d3d12` and
//!    `dxgi` in `<wine>/lib/wine/x86_64-windows/`;
//! 3. `<wine>/lib/wine/x86_64-unix/{d3d10,d3d11,d3d12,dxgi}.so` become symlinks
//!    to `../../external/libd3dshared.dylib`.
//!
//! Replacing the builtins in the engine itself would take Wine's own Direct3D
//! (and with it the DXVK pairing, which needs Wine's `dxgi`) away from every
//! bottle. Overriding them per process is not possible either: Wine searches its
//! own `lib/wine` before `WINEDLLPATH` (`set_dll_path()` in
//! `dlls/ntdll/unix/loader.c`), and a builtin-marked DLL in `system32` is
//! replaced by Wine's builtin. So the payload is deployed into an *overlay*: an
//! APFS clone of the engine under `engines/<id>/d3dmetal/`, which costs almost
//! no disk space. Programs whose backend is D3DMetal run the overlay's `wine`;
//! everything else keeps running the untouched engine.
//!
//! Only CrossOver-derived Wine builds can execute the payload. On upstream Wine
//! (our catalog's Gcenx builds) it loads, then every process that runs D3DMetal
//! code dies (frankea/Whisky#163: `libd3dshared.dylib` patches CrossOver's
//! unixcall table). Engines therefore have to be marked `gptk_capable` in the
//! catalog; `force` exists for users who bring such a build themselves.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::gptk::{self, Payload, FORWARDERS};
use crate::manager::{EngineManager, InstalledEngine, COMPONENTS_DIR, ENGINE_FILE};
use crate::{Error, Result};

/// Overlay directory inside an engine directory.
pub const OVERLAY_DIR: &str = "d3dmetal";
/// Record of what was deployed, inside the overlay directory.
pub const OVERLAY_FILE: &str = "overlay.toml";

/// What `overlay.toml` records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverlayRecord {
    /// GPTK version of the deployed payload, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gptk_version: Option<String>,
    /// Built with `force` on an engine not marked `gptk_capable`.
    #[serde(default)]
    pub forced: bool,
    pub created_unix: u64,
}

/// A D3DMetal overlay of an installed engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overlay {
    pub dir: PathBuf,
    /// The overlay's `wine` executable.
    pub wine: PathBuf,
    pub record: OverlayRecord,
}

impl Overlay {
    /// The overlay was built from a different GPTK import than `payload`.
    pub fn is_stale(&self, payload: Option<&Payload>) -> bool {
        payload.map_or(true, |p| p.version != self.record.gptk_version)
    }
}

/// `<prefix>/lib/wine` for a `wine` executable at `<prefix>/bin/wine`.
fn lib_wine_dir(wine: &Path) -> Option<PathBuf> {
    Some(wine.parent()?.parent()?.join("lib/wine"))
}

impl EngineManager<'_> {
    pub fn d3dmetal_dir(&self, engine_id: &str) -> PathBuf {
        self.engine_dir(engine_id).join(OVERLAY_DIR)
    }

    /// The engine's D3DMetal overlay, if one was built.
    pub fn d3dmetal_overlay(&self, engine: &InstalledEngine) -> Result<Option<Overlay>> {
        let dir = self.d3dmetal_dir(&engine.id);
        let file = dir.join(OVERLAY_FILE);
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::io(file, e)),
        };
        let record =
            toml::from_str(&text).map_err(|source| Error::TomlParse { path: file, source })?;
        Ok(Some(Overlay {
            wine: dir.join(&engine.wine_bin),
            dir,
            record,
        }))
    }

    /// Build (or rebuild) the D3DMetal overlay of `engine_id` from the GPTK
    /// import in `gptk_dir`. Refuses engines not marked `gptk_capable` unless
    /// `force` is set.
    pub fn enable_d3dmetal(
        &self,
        engine_id: &str,
        gptk_dir: &Path,
        force: bool,
    ) -> Result<Overlay> {
        let engine = self
            .get(engine_id)?
            .ok_or_else(|| Error::NotInstalled(engine_id.to_owned()))?;
        if !engine.gptk_capable && !force {
            return Err(Error::NotGptkCapable(engine_id.to_owned()));
        }
        let payload =
            gptk::inspect(gptk_dir).ok_or_else(|| Error::GptkNotFound(gptk_dir.to_owned()))?;
        payload.require_complete()?;
        let engine_dir = self.engine_dir(engine_id);
        if lib_wine_dir(&engine_dir.join(&engine.wine_bin))
            .map_or(true, |d| !d.join("x86_64-windows").is_dir())
        {
            return Err(Error::UnsupportedLayout {
                engine: engine_id.to_owned(),
            });
        }

        let staging = engine_dir.join(format!(".{OVERLAY_DIR}.building"));
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|e| Error::io(&staging, e))?;
        }
        let record = OverlayRecord {
            gptk_version: payload.version.clone(),
            forced: !engine.gptk_capable,
            created_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        };
        let built = build_overlay(&engine_dir, &staging, &engine, &payload, &record);
        if let Err(e) = built {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
        let target = self.d3dmetal_dir(engine_id);
        if target.exists() {
            fs::remove_dir_all(&target).map_err(|e| Error::io(&target, e))?;
        }
        fs::rename(&staging, &target).map_err(|e| Error::io(&target, e))?;
        tracing::info!(
            engine = engine_id,
            gptk = ?record.gptk_version,
            forced = record.forced,
            "D3DMetal overlay built"
        );
        Ok(Overlay {
            wine: target.join(&engine.wine_bin),
            dir: target,
            record,
        })
    }

    /// Delete the D3DMetal overlay; `false` if there was none.
    pub fn disable_d3dmetal(&self, engine_id: &str) -> Result<bool> {
        let dir = self.d3dmetal_dir(engine_id);
        if !dir.exists() {
            return Ok(false);
        }
        fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        tracing::info!(engine = engine_id, "D3DMetal overlay removed");
        Ok(true)
    }
}

fn build_overlay(
    engine_dir: &Path,
    staging: &Path,
    engine: &InstalledEngine,
    payload: &Payload,
    record: &OverlayRecord,
) -> Result<()> {
    fs::create_dir_all(staging).map_err(|e| Error::io(staging, e))?;
    // Clone the Wine tree, not our own bookkeeping.
    let entries = fs::read_dir(engine_dir).map_err(|e| Error::io(engine_dir, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(engine_dir, e))?;
        let name = entry.file_name();
        let skip = name == ENGINE_FILE
            || name == COMPONENTS_DIR
            || name == OVERLAY_DIR
            || name.to_string_lossy().starts_with('.');
        if !skip {
            clone_tree(&entry.path(), &staging.join(&name))?;
        }
    }

    let lib_wine =
        lib_wine_dir(&staging.join(&engine.wine_bin)).ok_or_else(|| Error::UnsupportedLayout {
            engine: engine.id.clone(),
        })?;
    let windows = lib_wine.join("x86_64-windows");
    for name in FORWARDERS {
        let to = windows.join(name);
        // Remove first: the clone shares blocks with the engine, a fresh copy must not.
        if to.symlink_metadata().is_ok() {
            fs::remove_file(&to).map_err(|e| Error::io(&to, e))?;
        }
        let from = payload.windows_dir().join(name);
        fs::copy(&from, &to).map_err(|e| Error::io(&to, e))?;
    }
    gptk::link_unix_bridges(&lib_wine.join("x86_64-unix"), &windows)?;

    let external = lib_wine
        .parent()
        .map(|lib| lib.join("external"))
        .ok_or_else(|| Error::UnsupportedLayout {
            engine: engine.id.clone(),
        })?;
    if external.exists() {
        fs::remove_dir_all(&external).map_err(|e| Error::io(&external, e))?;
    }
    clone_tree(&payload.external_dir(), &external)?;

    let file = staging.join(OVERLAY_FILE);
    fs::write(&file, toml::to_string_pretty(record)?).map_err(|e| Error::io(file, e))
}

/// `cp -cR` (clonefile on APFS, which keeps the overlay nearly free), falling
/// back to `cp -R` when cloning is impossible. Both keep symlinks as symlinks.
fn clone_tree(src: &Path, dest: &Path) -> Result<()> {
    let attempt = |flags: &str| {
        Command::new("cp")
            .arg(flags)
            .arg(src)
            .arg(dest)
            .output()
            .map_err(|e| Error::Command {
                program: "cp".into(),
                message: e.to_string(),
            })
    };
    let cloned = attempt("-cR")?;
    if cloned.status.success() {
        return Ok(());
    }
    if dest.symlink_metadata().is_ok() {
        let _ = fs::remove_dir_all(dest).or_else(|_| fs::remove_file(dest));
    }
    let copied = attempt("-R")?;
    if copied.status.success() {
        Ok(())
    } else {
        Err(Error::Command {
            program: "cp".into(),
            message: String::from_utf8_lossy(&copied.stderr).trim().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use cross_core::paths::AppDirs;

    use super::*;
    use crate::gptk::tests::{fake_builtin_dll, fake_full_toolkit};

    /// An installed engine with a Gcenx-like `.app` layout and Wine's own DLLs.
    fn fake_engine(dirs: &AppDirs, id: &str, gptk_capable: bool) -> InstalledEngine {
        let root = dirs.engine_dir(id);
        let wine = root.join("Wine.app/Contents/Resources/wine");
        let windows = wine.join("lib/wine/x86_64-windows");
        fs::create_dir_all(&windows).unwrap();
        fs::create_dir_all(wine.join("lib/wine/x86_64-unix")).unwrap();
        fs::create_dir_all(wine.join("bin")).unwrap();
        fs::write(wine.join("bin/wine"), "#!/bin/sh\necho overlay-or-not\n").unwrap();
        for name in FORWARDERS.iter().chain(&["d3d9.dll"]) {
            fs::write(
                windows.join(name),
                fake_builtin_dll(&format!("wine-{name}")),
            )
            .unwrap();
        }
        fs::write(wine.join("lib/wine/x86_64-unix/wined3d.so"), "wined3d").unwrap();
        fs::create_dir_all(root.join(COMPONENTS_DIR).join("dxvk")).unwrap();
        let engine = InstalledEngine {
            id: id.into(),
            name: id.into(),
            wine_version: "11.0".into(),
            translation: Default::default(),
            wine_bin: "Wine.app/Contents/Resources/wine/bin/wine".into(),
            backends: vec![],
            components: vec!["dxvk".into()],
            gptk_capable,
            source_sha256: "0".repeat(64),
            installed_unix: 1,
        };
        fs::write(
            root.join(ENGINE_FILE),
            toml::to_string_pretty(&engine).unwrap(),
        )
        .unwrap();
        engine
    }

    fn imported_gptk(tmp: &Path) -> PathBuf {
        let src = tmp.join("Game_Porting_Toolkit_4.0");
        fake_full_toolkit(&src);
        let dest = tmp.join("home/gptk");
        gptk::import(&src, &dest, None).unwrap();
        dest
    }

    #[test]
    fn overlay_gets_the_payload_and_leaves_the_engine_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        let engine = fake_engine(&dirs, "cx", true);
        let gptk_dir = imported_gptk(tmp.path());
        let manager = EngineManager::new(&dirs);

        assert_eq!(manager.d3dmetal_overlay(&engine).unwrap(), None);
        let overlay = manager.enable_d3dmetal("cx", &gptk_dir, false).unwrap();
        assert!(!overlay.record.forced);
        assert_eq!(overlay.record.gptk_version.as_deref(), Some("4.0"));
        assert_eq!(
            manager.d3dmetal_overlay(&engine).unwrap(),
            Some(overlay.clone())
        );
        assert!(overlay.wine.is_file());

        let lib = overlay.dir.join("Wine.app/Contents/Resources/wine/lib");
        let ours = fs::read(lib.join("wine/x86_64-windows/d3d12.dll")).unwrap();
        assert!(ours.ends_with(b"d3d12.dll") && !ours.ends_with(b"wine-d3d12.dll"));
        // d3d9 is not one of Apple's forwarders and stays Wine's.
        assert!(fs::read(lib.join("wine/x86_64-windows/d3d9.dll"))
            .unwrap()
            .ends_with(b"wine-d3d9.dll"));
        assert_eq!(
            fs::read_link(lib.join("wine/x86_64-unix/dxgi.so")).unwrap(),
            Path::new(gptk::UNIX_LINK_TARGET)
        );
        assert_eq!(
            fs::read_to_string(lib.join("wine/x86_64-unix/dxgi.so")).unwrap(),
            "lib",
            "the bridge resolves to libd3dshared.dylib inside the overlay"
        );
        assert!(lib.join("external/D3DMetal.framework/D3DMetal").is_file());
        assert!(lib.join("wine/x86_64-unix/wined3d.so").is_file());
        // Bookkeeping is not cloned.
        assert!(!overlay.dir.join(COMPONENTS_DIR).exists());
        assert!(!overlay.dir.join(ENGINE_FILE).exists());

        // The engine itself is untouched.
        let engine_lib = dirs
            .engine_dir("cx")
            .join("Wine.app/Contents/Resources/wine/lib");
        assert!(fs::read(engine_lib.join("wine/x86_64-windows/d3d12.dll"))
            .unwrap()
            .ends_with(b"wine-d3d12.dll"));
        assert!(!engine_lib.join("external").exists());
        assert!(!engine_lib.join("wine/x86_64-unix/dxgi.so").exists());

        // Rebuilding replaces the overlay; disabling removes it.
        manager.enable_d3dmetal("cx", &gptk_dir, false).unwrap();
        assert!(!dirs.engine_dir("cx").join(".d3dmetal.building").exists());
        assert!(manager.disable_d3dmetal("cx").unwrap());
        assert!(!manager.disable_d3dmetal("cx").unwrap());
        assert_eq!(manager.d3dmetal_overlay(&engine).unwrap(), None);
    }

    #[test]
    fn upstream_engines_need_force() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        fake_engine(&dirs, "upstream", false);
        let gptk_dir = imported_gptk(tmp.path());
        let manager = EngineManager::new(&dirs);

        let err = manager
            .enable_d3dmetal("upstream", &gptk_dir, false)
            .unwrap_err();
        assert!(matches!(err, Error::NotGptkCapable(_)), "{err}");
        assert!(!manager.d3dmetal_dir("upstream").exists());

        let forced = manager
            .enable_d3dmetal("upstream", &gptk_dir, true)
            .unwrap();
        assert!(forced.record.forced);
    }

    #[test]
    fn incomplete_or_missing_gptk_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        fake_engine(&dirs, "cx", true);
        let manager = EngineManager::new(&dirs);

        let err = manager
            .enable_d3dmetal("cx", &tmp.path().join("nothing"), false)
            .unwrap_err();
        assert!(matches!(err, Error::GptkNotFound(_)), "{err}");

        let gptk_dir = imported_gptk(tmp.path());
        fs::remove_file(gptk_dir.join("lib/wine/x86_64-windows/dxgi.dll")).unwrap();
        let err = manager.enable_d3dmetal("cx", &gptk_dir, false).unwrap_err();
        assert!(matches!(err, Error::GptkIncomplete { .. }), "{err}");

        let err = manager
            .enable_d3dmetal("nope", &gptk_dir, false)
            .unwrap_err();
        assert!(matches!(err, Error::NotInstalled(_)), "{err}");
    }

    #[test]
    fn staleness_follows_the_import_version() {
        let overlay = Overlay {
            dir: "/x".into(),
            wine: "/x/bin/wine".into(),
            record: OverlayRecord {
                gptk_version: Some("4.0".into()),
                forced: false,
                created_unix: 0,
            },
        };
        let payload = |v: &str| Payload {
            dir: "/g".into(),
            version: Some(v.into()),
            missing: vec![],
        };
        assert!(!overlay.is_stale(Some(&payload("4.0"))));
        assert!(overlay.is_stale(Some(&payload("4.1"))));
        assert!(overlay.is_stale(None));
    }
}
