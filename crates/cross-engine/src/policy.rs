//! What an engine's [`Translation`] means for installing and running it.
//!
//! - `rosetta`: x86_64 Wine. Needs Rosetta 2 to run; on macOS 28+ Rosetta is
//!   kept only for legacy games, which Wine is not known to qualify for.
//! - `native`: arm64 Wine with its own x86 emulator. Must not need Rosetta:
//!   the unpacked `wine` has to contain an arm64 slice.
//! - `external`: another product's Wine (CrossOver), found on the system.
//!
//! The rules live here so that a new engine only needs a manifest entry.

use std::path::Path;

use cross_core::model::Translation;
use cross_core::system::{self, MacVersion};

use crate::catalog::EngineEntry;
use crate::macho::{self, Arch};
use crate::manager::InstalledEngine;
use crate::{Error, Result};

/// First macOS where Rosetta 2 is limited to legacy games.
pub const ROSETTA_LIMITED_FROM: u32 = 28;

/// The facts about this Mac that the rules depend on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    /// `aarch64` on Apple Silicon.
    pub arch: String,
    pub macos: Option<MacVersion>,
    pub rosetta_installed: bool,
}

impl Host {
    pub fn detect() -> Self {
        Self {
            arch: std::env::consts::ARCH.to_owned(),
            macos: system::macos_version(),
            rosetta_installed: system::rosetta_installed(),
        }
    }

    fn rosetta_limited(&self) -> bool {
        self.macos.is_some_and(|v| v.major >= ROSETTA_LIMITED_FROM)
    }
}

/// Hard requirements checked before installing: a native engine needs Apple
/// Silicon and its minimum macOS.
pub fn check_install(entry: &EngineEntry, host: &Host) -> Result<()> {
    if entry.translation == Translation::Native && host.arch != "aarch64" {
        return Err(Error::Unsupported {
            id: entry.id.clone(),
            requirement: "an Apple Silicon Mac".into(),
            found: host.arch.clone(),
        });
    }
    if let (Some(min), Some(found)) = (entry.min_macos(), host.macos) {
        if found < min {
            return Err(Error::Unsupported {
                id: entry.id.clone(),
                requirement: format!("macOS {min} or newer"),
                found: format!("macOS {found}"),
            });
        }
    }
    Ok(())
}

/// Things worth telling the user before installing, which do not stop it.
pub fn install_warnings(entry: &EngineEntry, host: &Host) -> Vec<String> {
    let mut warnings = Vec::new();
    if entry.requires_rosetta() {
        if !host.rosetta_installed {
            warnings.push(format!(
                "{} needs Rosetta 2, which is not installed; run `cross engine install-rosetta`",
                entry.id
            ));
        }
        if host.rosetta_limited() {
            warnings.push(format!(
                "{} needs Rosetta 2; macOS {ROSETTA_LIMITED_FROM}+ keeps Rosetta only for \
                 legacy games and Wine is not known to qualify",
                entry.id
            ));
        }
    }
    warnings
}

/// After unpacking: a native engine's `wine` must run without Rosetta.
/// Files that are not Mach-O (wrapper scripts) are accepted with a log line.
pub fn verify_binary(id: &str, translation: Translation, wine: &Path) -> Result<()> {
    if translation != Translation::Native {
        return Ok(());
    }
    match macho::architectures(wine) {
        Some(archs) if !archs.contains(&Arch::Arm64) => Err(Error::NotNative {
            id: id.to_owned(),
            path: wine.to_owned(),
            found: archs.iter().map(|a| a.name()).collect::<Vec<_>>().join("+"),
        }),
        Some(_) => Ok(()),
        None => {
            tracing::warn!(engine = %id, wine = %wine.display(), "wine is not a Mach-O file; cannot check its architecture");
            Ok(())
        }
    }
}

/// Before running Wine from an installed engine.
pub fn check_run(engine: &InstalledEngine, wine: &Path, host: &Host) -> Result<()> {
    match engine.translation {
        Translation::Rosetta => {
            // Only a real x86_64-only binary needs Rosetta (test fakes are scripts).
            if macho::needs_rosetta(wine) {
                if !host.rosetta_installed {
                    return Err(Error::RosettaRequired {
                        id: engine.id.clone(),
                    });
                }
                if host.rosetta_limited() {
                    tracing::warn!(
                        engine = %engine.id,
                        "this engine runs through Rosetta 2, which macOS {ROSETTA_LIMITED_FROM}+ keeps only for legacy games"
                    );
                }
            }
            Ok(())
        }
        Translation::Native => verify_binary(&engine.id, engine.translation, wine),
        Translation::External => {
            if !wine.exists() {
                return Err(Error::ExternalMissing {
                    id: engine.id.clone(),
                    path: wine.to_owned(),
                });
            }
            // The front end may be a script; the loader beside it tells whether
            // this release of the product still runs Wine through Rosetta.
            let intel = macho::needs_rosetta(wine)
                || macho::needs_rosetta(&crate::crossover::loader_of(wine));
            if intel && !host.rosetta_installed {
                return Err(Error::RosettaRequired {
                    id: engine.id.clone(),
                });
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::catalog::Catalog;
    use crate::macho::test_support::{thin_header, ARM64, X86_64};

    fn host(rosetta: bool, macos: (u32, u32)) -> Host {
        Host {
            arch: "aarch64".into(),
            macos: Some(MacVersion::new(macos.0, macos.1, 0)),
            rosetta_installed: rosetta,
        }
    }

    fn installed(translation: Translation) -> InstalledEngine {
        InstalledEngine {
            id: "e".into(),
            name: "E".into(),
            wine_version: "1".into(),
            translation,
            wine_bin: "bin/wine".into(),
            backends: vec![],
            components: vec![],
            gptk_capable: false,
            source_sha256: String::new(),
            installed_unix: 0,
        }
    }

    fn file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }

    fn native_entry() -> EngineEntry {
        let mut entry = Catalog::builtin()
            .engine("wine-devel-11.18")
            .unwrap()
            .clone();
        entry.translation = Translation::Native;
        entry.min_macos = Some("26.5".into());
        entry
    }

    #[test]
    fn rosetta_engine_needs_rosetta_only_for_x86_64_binaries() {
        let tmp = tempfile::tempdir().unwrap();
        let intel = file(tmp.path(), "intel", &thin_header(X86_64));
        let script = file(tmp.path(), "script", b"#!/bin/sh\n");
        let engine = installed(Translation::Rosetta);

        let err = check_run(&engine, &intel, &host(false, (26, 5))).unwrap_err();
        assert!(matches!(err, Error::RosettaRequired { .. }), "{err}");
        assert!(err.to_string().contains("install-rosetta"));
        check_run(&engine, &intel, &host(true, (26, 5))).unwrap();
        // Still allowed on macOS 28 (only a warning): Apple has not said what the subset covers.
        check_run(&engine, &intel, &host(true, (28, 0))).unwrap();
        check_run(&engine, &script, &host(false, (26, 5))).unwrap();
    }

    #[test]
    fn native_engine_must_not_need_rosetta() {
        let tmp = tempfile::tempdir().unwrap();
        let intel = file(tmp.path(), "intel", &thin_header(X86_64));
        let arm = file(tmp.path(), "arm", &thin_header(ARM64));
        let engine = installed(Translation::Native);

        let err = check_run(&engine, &intel, &host(true, (26, 5))).unwrap_err();
        assert!(matches!(err, Error::NotNative { .. }), "{err}");
        assert!(err.to_string().contains("x86_64"), "{err}");
        check_run(&engine, &arm, &host(false, (26, 5))).unwrap();
        verify_binary("e", Translation::Native, &arm).unwrap();
        // Rosetta engines are not held to this.
        verify_binary("e", Translation::Rosetta, &intel).unwrap();
    }

    #[test]
    fn external_engine_must_still_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let engine = installed(Translation::External);
        let wine = file(tmp.path(), "wine", b"#!/bin/sh\n");
        check_run(&engine, &wine, &host(false, (26, 5))).unwrap();

        // An x86_64-only loader beside the script front end (CrossOver 26.2) needs Rosetta.
        file(tmp.path(), "wineloader", &thin_header(X86_64));
        let err = check_run(&engine, &wine, &host(false, (26, 5))).unwrap_err();
        assert!(matches!(err, Error::RosettaRequired { .. }), "{err}");
        check_run(&engine, &wine, &host(true, (26, 5))).unwrap();
        // An arm64 loader (an ARM64 build) does not.
        file(tmp.path(), "wineloader", &thin_header(ARM64));
        check_run(&engine, &wine, &host(false, (26, 5))).unwrap();
        let err = check_run(&engine, &tmp.path().join("gone"), &host(false, (26, 5))).unwrap_err();
        assert!(matches!(err, Error::ExternalMissing { .. }), "{err}");
    }

    #[test]
    fn native_install_needs_apple_silicon_and_min_macos() {
        let entry = native_entry();
        check_install(&entry, &host(false, (26, 5))).unwrap();
        let err = check_install(&entry, &host(false, (26, 4))).unwrap_err();
        assert!(err.to_string().contains("macOS 26.5.0 or newer"), "{err}");

        let mut intel = host(true, (26, 5));
        intel.arch = "x86_64".into();
        assert!(check_install(&entry, &intel).is_err());
    }

    #[test]
    fn rosetta_install_only_warns() {
        let entry = Catalog::builtin()
            .engine("wine-devel-11.18")
            .unwrap()
            .clone();
        check_install(&entry, &host(false, (28, 0))).unwrap();
        assert!(install_warnings(&entry, &host(true, (26, 5))).is_empty());
        let warnings = install_warnings(&entry, &host(false, (28, 0)));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(install_warnings(&native_entry(), &host(false, (28, 0))).is_empty());
    }
}
