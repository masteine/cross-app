//! Importing CrossOver bottles so cross-app can run programs in them through
//! CrossOver's own Wine (an `external` engine).
//!
//! An imported bottle is a link, not a copy: `bottles/<id>/prefix` is a
//! symlink to the CrossOver bottle, which CrossOver keeps owning. Deleting the
//! cross-app bottle removes only the link; cloning it is refused.

use std::fs;
use std::os::unix::fs::symlink;
use std::time::{SystemTime, UNIX_EPOCH};

use cross_core::config::BottleStore;
use cross_core::model::{validate_bottle_id, Bottle, Translation};
use cross_core::paths::AppDirs;
use cross_engine::crossover::CrossOverBottle;
use cross_engine::manager::EngineManager;

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// cross-app bottle id; defaults to [`suggested_id`] of the CrossOver name.
    pub id: Option<String>,
    /// Display name; defaults to the CrossOver bottle name.
    pub name: Option<String>,
    /// Registered external engine to run it with (`crossover`).
    pub engine: String,
}

/// A path-safe bottle id for a CrossOver bottle name such as `Steam Games`.
pub fn suggested_id(name: &str) -> String {
    let mut id = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' {
            id.push(c);
        } else if !id.ends_with('-') {
            id.push('-');
        }
    }
    let id = id.trim_matches(|c| c == '-' || c == '_');
    let id: String = id.chars().take(64).collect();
    let id = id.trim_end_matches('-');
    if id.is_empty() {
        "crossover".to_owned()
    } else {
        id.to_owned()
    }
}

/// Link a CrossOver bottle into cross-app. Nothing in the CrossOver bottle changes.
pub fn import(dirs: &AppDirs, source: &CrossOverBottle, options: &ImportOptions) -> Result<Bottle> {
    let id = options
        .id
        .clone()
        .unwrap_or_else(|| suggested_id(&source.name));
    validate_bottle_id(&id)?;
    let engine = EngineManager::new(dirs)
        .get(&options.engine)?
        .ok_or_else(|| Error::EngineNotInstalled(options.engine.clone()))?;
    if engine.translation != Translation::External {
        return Err(Error::InvalidInput(format!(
            "engine {:?} is not an external engine; register CrossOver with `cross engine install crossover`",
            options.engine
        )));
    }
    let target = source
        .path
        .canonicalize()
        .map_err(|e| Error::io(&source.path, e))?;
    let dir = dirs.bottle_dir(&id);
    if dir.exists() {
        return Err(Error::BottleExists(id));
    }

    let result = link_into(dirs, &id, &target, source, options);
    if result.is_err() {
        // Only our directory and the link in it: never the CrossOver bottle.
        let _ = fs::remove_dir_all(&dir);
    }
    result
}

fn link_into(
    dirs: &AppDirs,
    id: &str,
    target: &std::path::Path,
    source: &CrossOverBottle,
    options: &ImportOptions,
) -> Result<Bottle> {
    let dir = dirs.bottle_dir(id);
    fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let prefix = dirs.bottle_prefix(id);
    symlink(target, &prefix).map_err(|e| Error::io(&prefix, e))?;

    let mut bottle = Bottle::new(id, options.name.as_deref().unwrap_or(&source.name))?;
    bottle.engine = Some(options.engine.clone());
    bottle.created_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    BottleStore::new(dirs).save(&bottle)?;
    tracing::info!(bottle = %id, engine = %options.engine, crossover_bottle = %source.name, "imported CrossOver bottle");
    Ok(bottle)
}

/// Whether a bottle's prefix is a link to a bottle owned by another product.
pub fn is_linked(dirs: &AppDirs, id: &str) -> bool {
    fs::symlink_metadata(dirs.bottle_prefix(id)).is_ok_and(|m| m.file_type().is_symlink())
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    use cross_engine::manager::{InstalledEngine, ENGINE_FILE};

    use super::*;

    pub struct FakeCrossOver {
        pub wine: PathBuf,
        pub log: PathBuf,
        pub bottles: PathBuf,
    }

    impl FakeCrossOver {
        pub fn calls(&self) -> String {
            fs::read_to_string(&self.log).unwrap_or_default()
        }
    }

    /// A fake `CrossOver.app` (recording `bin/wine`), registered in `dirs` as
    /// the external engine `crossover`, with one bottle `name`.
    pub fn fake_crossover(dirs: &AppDirs, root: &Path, name: &str) -> FakeCrossOver {
        let bin = root.join("CrossOver.app/Contents/SharedSupport/CrossOver/bin");
        fs::create_dir_all(&bin).unwrap();
        let wine = bin.join("wine");
        fs::write(
            &wine,
            "#!/bin/sh\nhere=\"$(dirname \"$0\")\"\necho \"ARGS:$*\" >> \"$here/calls.log\"\n\
             echo \"PREFIX:$WINEPREFIX\" >> \"$here/calls.log\"\n\
             echo \"CX:$CX_ROOT|$CX_BOTTLE|$CX_BOTTLE_PATH\" >> \"$here/calls.log\"\nexit 0\n",
        )
        .unwrap();
        fs::set_permissions(&wine, fs::Permissions::from_mode(0o755)).unwrap();

        let engine = InstalledEngine {
            id: "crossover".into(),
            name: "CrossOver".into(),
            wine_version: "27.0".into(),
            translation: Translation::External,
            wine_bin: wine.clone(),
            backends: vec![],
            components: vec![],
            source_sha256: String::new(),
            installed_unix: 0,
        };
        let dir = dirs.engine_dir("crossover");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(ENGINE_FILE), toml::to_string(&engine).unwrap()).unwrap();

        let bottles = root.join("Bottles");
        let bottle = bottles.join(name);
        fs::create_dir_all(bottle.join("drive_c/windows")).unwrap();
        fs::write(
            bottle.join("system.reg"),
            "WINE REGISTRY Version 2\n\n#arch=win64\n",
        )
        .unwrap();
        FakeCrossOver {
            log: bin.join("calls.log"),
            wine,
            bottles,
        }
    }
}

#[cfg(test)]
mod tests {
    use cross_engine::crossover::find_bottle;

    use super::test_support::fake_crossover;
    use super::*;
    use crate::launch::{self, LaunchRequest};
    use crate::manager::BottleManager;
    use crate::runtime::Runtime;

    fn options(id: Option<&str>) -> ImportOptions {
        ImportOptions {
            id: id.map(str::to_owned),
            name: None,
            engine: "crossover".into(),
        }
    }

    #[test]
    fn suggested_ids_are_valid_bottle_ids() {
        for (name, id) in [
            ("Steam", "steam"),
            ("Steam Games 2", "steam-games-2"),
            ("  Old Office (32-bit)!", "old-office-32-bit"),
            ("Привіт", "crossover"),
            ("__x__", "x"),
        ] {
            assert_eq!(suggested_id(name), id, "{name:?}");
            validate_bottle_id(&suggested_id(name)).unwrap();
        }
        validate_bottle_id(&suggested_id(&"a b".repeat(40))).unwrap();
    }

    #[test]
    fn import_links_and_launches_through_crossover_wine() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        let fake = fake_crossover(&dirs, tmp.path(), "Steam Games");
        let source = find_bottle(std::slice::from_ref(&fake.bottles), "Steam Games").unwrap();

        let bottle = import(&dirs, &source, &options(None)).unwrap();
        assert_eq!(bottle.id, "steam-games");
        assert_eq!(bottle.name, "Steam Games");
        assert_eq!(bottle.engine.as_deref(), Some("crossover"));
        assert!(is_linked(&dirs, "steam-games"));

        let (_, runtime) = Runtime::open(&dirs, "steam-games").unwrap();
        let real = source.path.canonicalize().unwrap();
        assert_eq!(runtime.prefix(), real);
        let status = launch::run(
            &runtime,
            &LaunchRequest {
                target: "notepad".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(status.success());
        let calls = fake.calls();
        assert!(calls.contains("ARGS:notepad"), "{calls}");
        assert!(
            calls.contains(&format!("PREFIX:{}", real.display())),
            "{calls}"
        );
        let cx_root = fake.wine.parent().unwrap().parent().unwrap();
        assert!(
            calls.contains(&format!(
                "CX:{}|Steam Games|{}",
                cx_root.display(),
                real.parent().unwrap().display()
            )),
            "{calls}"
        );
    }

    #[test]
    fn deleting_an_imported_bottle_keeps_the_crossover_bottle() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        let fake = fake_crossover(&dirs, tmp.path(), "Steam");
        let source = find_bottle(std::slice::from_ref(&fake.bottles), "Steam").unwrap();
        import(&dirs, &source, &options(Some("steam"))).unwrap();

        BottleManager::new(&dirs).delete("steam").unwrap();
        assert!(!dirs.bottle_dir("steam").exists());
        assert!(source.path.join("system.reg").is_file());
        assert!(source.path.join("drive_c/windows").is_dir());
    }

    #[test]
    fn import_refuses_duplicates_bad_ids_and_non_external_engines() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        let fake = fake_crossover(&dirs, tmp.path(), "Steam");
        let source = find_bottle(std::slice::from_ref(&fake.bottles), "Steam").unwrap();
        import(&dirs, &source, &options(Some("steam"))).unwrap();

        assert!(matches!(
            import(&dirs, &source, &options(Some("steam"))).unwrap_err(),
            Error::BottleExists(_)
        ));
        assert!(matches!(
            import(&dirs, &source, &options(Some("Bad Id"))).unwrap_err(),
            Error::Core(_)
        ));
        let mut missing = options(Some("other"));
        missing.engine = "nope".into();
        assert!(matches!(
            import(&dirs, &source, &missing).unwrap_err(),
            Error::EngineNotInstalled(_)
        ));
        assert!(!dirs.bottle_dir("other").exists());
    }

    #[test]
    fn imported_bottles_cannot_be_cloned_and_external_engines_cannot_create() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        let fake = fake_crossover(&dirs, tmp.path(), "Steam");
        let source = find_bottle(std::slice::from_ref(&fake.bottles), "Steam").unwrap();
        import(&dirs, &source, &options(Some("steam"))).unwrap();
        let manager = BottleManager::new(&dirs);

        let err = manager.clone_bottle("steam", "copy", None).unwrap_err();
        assert!(err.to_string().contains("CrossOver"), "{err}");
        assert!(!dirs.bottle_dir("copy").exists());

        let err = manager
            .create(&crate::manager::CreateOptions {
                id: "fresh".into(),
                name: None,
                engine: "crossover".into(),
                windows: cross_core::model::WindowsVersion::Win10,
            })
            .unwrap_err();
        assert!(err.to_string().contains("import"), "{err}");
        assert!(!dirs.bottle_dir("fresh").exists());

        // Renaming moves only our record; the link keeps pointing at CrossOver's bottle.
        manager.rename("steam", "games").unwrap();
        let (_, runtime) = Runtime::open(&dirs, "games").unwrap();
        assert_eq!(runtime.prefix(), source.path.canonicalize().unwrap());
    }

    #[test]
    fn removed_crossover_is_reported_on_open() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        let fake = fake_crossover(&dirs, tmp.path(), "Steam");
        let source = find_bottle(std::slice::from_ref(&fake.bottles), "Steam").unwrap();
        import(&dirs, &source, &options(Some("steam"))).unwrap();
        fs::remove_file(&fake.wine).unwrap();
        let err = Runtime::open(&dirs, "steam").unwrap_err();
        assert!(err.to_string().contains("no longer exists"), "{err}");
    }
}
