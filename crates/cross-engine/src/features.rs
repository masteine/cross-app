//! What an installed Wine build actually supports, found by looking at its files.
//!
//! Several switches only exist in patched builds, and setting them on a build
//! that lacks the patch does nothing at all. Rather than trusting a catalog flag
//! we look for the string the code passes to `getenv`/the registry in the unix
//! libraries of the engine:
//!
//! | feature | string | read by | where it comes from |
//! |---|---|---|---|
//! | esync | `WINEESYNC` | `ntdll.so` | esync patches (wine-staging history, CrossOver; `README.esync` in Gcenx/game-porting-toolkit) |
//! | msync | `WINEMSYNC` | `ntdll.so` | marzent/wine-msync, carried by CrossOver-derived macOS builds |
//! | CPU count | `WINENCPU` | `ntdll.so` | `get_cpu_count_override()` in `dlls/ntdll/unix/system.c` of Apple's GPTK Wine (Gcenx/game-porting-toolkit); not in upstream Wine 11.18 |
//! | Retina mode | `RetinaMode` | `winemac.so` | `setup_options()` in upstream `dlls/winemac.drv/macdrv_main.c` |
//!
//! A string match is evidence, not proof (the string could appear for another
//! reason), so callers present these as "detected". Upstream Wine 11.18 from
//! Gcenx/macOS_Wine_builds has `RetinaMode` but none of the other three
//! (checked on the real `wine-devel-11.18` engine, 15.10.2026).

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Switches detected in an engine's unix libraries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct EngineFeatures {
    /// `ntdll.so` was found and inspected; when false every flag is unknown.
    pub inspected: bool,
    pub esync: bool,
    pub msync: bool,
    /// `WINENCPU` overrides the processor count reported to programs.
    pub cpu_count_override: bool,
    /// `HKCU\Software\Wine\Mac Driver\RetinaMode` is read by the Mac driver.
    pub retina_mode: bool,
}

/// Directory with the unix-side libraries for a `wine` executable laid out as
/// `<prefix>/bin/wine` + `<prefix>/lib/wine/x86_64-unix/`.
pub fn unix_lib_dir(wine: &Path) -> Option<PathBuf> {
    let prefix = wine.parent()?.parent()?;
    ["lib/wine/x86_64-unix", "lib64/wine/x86_64-unix"]
        .iter()
        .map(|rel| prefix.join(rel))
        .find(|dir| dir.join("ntdll.so").is_file())
}

/// Inspect the engine whose `wine` executable is at `wine`.
pub fn probe(wine: &Path) -> EngineFeatures {
    let Some(dir) = unix_lib_dir(wine) else {
        return EngineFeatures::default();
    };
    let Ok(ntdll) = fs::read(dir.join("ntdll.so")) else {
        return EngineFeatures::default();
    };
    let winemac = fs::read(dir.join("winemac.so")).unwrap_or_default();
    EngineFeatures {
        inspected: true,
        esync: contains(&ntdll, b"WINEESYNC"),
        msync: contains(&ntdll, b"WINEMSYNC"),
        cpu_count_override: contains(&ntdll, b"WINENCPU"),
        retina_mode: contains(&winemac, b"RetinaMode"),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    let Some((&first, rest)) = needle.split_first() else {
        return true;
    };
    haystack
        .iter()
        .enumerate()
        .filter(|(_, &b)| b == first)
        .any(|(i, _)| haystack[i + 1..].starts_with(rest))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fake engine whose unix libraries contain `ntdll` and `winemac` bytes.
    pub(crate) fn fake_engine(root: &Path, ntdll: &[u8], winemac: &[u8]) -> PathBuf {
        let unix = root.join("wine/lib/wine/x86_64-unix");
        fs::create_dir_all(&unix).unwrap();
        fs::create_dir_all(root.join("wine/bin")).unwrap();
        fs::write(unix.join("ntdll.so"), ntdll).unwrap();
        fs::write(unix.join("winemac.so"), winemac).unwrap();
        let wine = root.join("wine/bin/wine");
        fs::write(&wine, "#!/bin/sh\n").unwrap();
        wine
    }

    #[test]
    fn substring_search() {
        assert!(contains(b"xxWINEMSYNCyy", b"WINEMSYNC"));
        assert!(!contains(b"WINEMSYN", b"WINEMSYNC"));
        assert!(contains(b"abc", b""));
        assert!(!contains(b"", b"a"));
    }

    #[test]
    fn detects_patched_and_plain_builds() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = fake_engine(&tmp.path().join("a"), b"\0WINEDLLPATH\0", b"\0RetinaMode\0");
        assert_eq!(
            probe(&plain),
            EngineFeatures {
                inspected: true,
                retina_mode: true,
                ..Default::default()
            }
        );
        let patched = fake_engine(
            &tmp.path().join("b"),
            b"\0WINEESYNC\0WINEMSYNC\0WINENCPU\0",
            b"",
        );
        let f = probe(&patched);
        assert!(f.esync && f.msync && f.cpu_count_override && !f.retina_mode);
        assert_eq!(
            probe(&tmp.path().join("none/bin/wine")),
            EngineFeatures::default()
        );
    }

    /// Checks the claims in the module docs against a real engine:
    /// `CROSS_TEST_ENGINES_HOME=<dir with engines/> cargo test -p cross-engine -- --ignored`
    #[test]
    #[ignore = "needs a real engine in CROSS_TEST_ENGINES_HOME"]
    fn real_upstream_engine_has_retina_but_no_sync_patches() {
        let home = std::env::var_os("CROSS_TEST_ENGINES_HOME").expect("CROSS_TEST_ENGINES_HOME");
        let dirs = cross_core::paths::AppDirs::with_root(home);
        let manager = crate::manager::EngineManager::new(&dirs);
        let engine = manager
            .get("wine-devel-11.18")
            .unwrap()
            .expect("wine-devel-11.18 installed");
        let f = probe(&manager.wine_path(&engine));
        assert!(f.inspected);
        assert!(f.retina_mode);
        assert!(!f.esync && !f.msync && !f.cpu_count_override, "{f:?}");
    }
}
