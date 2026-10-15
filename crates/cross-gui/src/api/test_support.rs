//! Throwaway data directories for the command-layer tests: a fake engine whose
//! `wine` is a shell script, and bottles that point at it.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use cross_core::config::BottleStore;
use cross_core::model::{Bottle, Translation};
use cross_core::paths::AppDirs;
use cross_engine::manager::{InstalledEngine, ENGINE_FILE};

/// `wine` prints its arguments (one `ARG:` line each) and its working
/// directory, then exits 0. `wineserver` does nothing.
const FAKE_WINE: &str = r#"#!/bin/sh
for a in "$@"; do echo "ARG:$a"; done
echo "CWD:$(pwd)"
echo "PREFIX:$WINEPREFIX"
exit 0
"#;

/// Install a fake engine `id` and return the path of its `wine`.
pub fn fake_engine(dirs: &AppDirs, id: &str) -> PathBuf {
    let dir = dirs.engine_dir(id);
    let bin = dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    for (name, script) in [("wine", FAKE_WINE), ("wineserver", "#!/bin/sh\nexit 0\n")] {
        let path = bin.join(name);
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let engine = InstalledEngine {
        id: id.into(),
        name: format!("Fake {id}"),
        wine_version: "11.0".into(),
        translation: Translation::Rosetta,
        wine_bin: "bin/wine".into(),
        backends: vec![],
        components: vec![],
        source_sha256: "0".repeat(64),
        installed_unix: 1,
    };
    fs::write(
        dir.join(ENGINE_FILE),
        toml::to_string_pretty(&engine).unwrap(),
    )
    .unwrap();
    bin.join("wine")
}

/// A bottle with an (empty) prefix that uses the fake engine `engine`.
pub fn fake_bottle(dirs: &AppDirs, id: &str, engine: &str) -> Bottle {
    if !dirs.engine_dir(engine).exists() {
        fake_engine(dirs, engine);
    }
    fs::create_dir_all(dirs.bottle_prefix(id).join("drive_c")).unwrap();
    let mut bottle = Bottle::new(id, id.to_uppercase()).unwrap();
    bottle.engine = Some(engine.into());
    BottleStore::new(dirs).save(&bottle).unwrap();
    bottle
}
