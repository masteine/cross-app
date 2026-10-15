//! Creating, listing, cloning, renaming and deleting bottles.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use cross_core::config::{BottleStore, Config};
use cross_core::model::{validate_bottle_id, Bottle, Translation, WindowsVersion};
use cross_core::paths::AppDirs;
use cross_engine::manager::EngineManager;

use crate::exec::{run_captured, run_checked};
use crate::processes::{running_wineservers, StopMode};
use crate::runtime::Runtime;
use crate::{Error, Result};

const WINEBOOT_TIMEOUT: Duration = Duration::from_secs(600);
const WINESERVER_WAIT_TIMEOUT: Duration = Duration::from_secs(120);
const COPY_TIMEOUT: Duration = Duration::from_secs(1800);
/// How long a bottle may take to go idle after its last program exits.
const IDLE_GRACE: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct CreateOptions {
    pub id: String,
    /// Display name; defaults to the id.
    pub name: Option<String>,
    pub engine: String,
    pub windows: WindowsVersion,
}

pub struct BottleManager<'a> {
    dirs: &'a AppDirs,
}

impl<'a> BottleManager<'a> {
    pub fn new(dirs: &'a AppDirs) -> Self {
        Self { dirs }
    }

    pub fn list(&self) -> Result<Vec<Bottle>> {
        Ok(BottleStore::new(self.dirs).list()?)
    }

    pub fn get(&self, id: &str) -> Result<Bottle> {
        validate_bottle_id(id)?;
        if !self.dirs.bottle_file(id).is_file() {
            return Err(Error::BottleNotFound(id.to_owned()));
        }
        Ok(BottleStore::new(self.dirs).load(id)?)
    }

    /// Pick the engine for a new bottle: the explicit one, else the configured
    /// default, else the only installed engine.
    pub fn resolve_engine(&self, explicit: Option<&str>, config: &Config) -> Result<String> {
        if let Some(id) = explicit.or(config.default_engine.as_deref()) {
            return Ok(id.to_owned());
        }
        let installed: Vec<String> = EngineManager::new(self.dirs)
            .list()?
            .into_iter()
            .map(|e| e.id)
            .collect();
        match installed.as_slice() {
            [only] => Ok(only.clone()),
            _ => Err(Error::NoEngineChosen { installed }),
        }
    }

    /// Create a bottle: initialise a 64-bit Wine prefix and set the Windows version.
    /// A failed creation leaves nothing behind.
    pub fn create(&self, options: &CreateOptions) -> Result<Bottle> {
        validate_bottle_id(&options.id)?;
        let dir = self.dirs.bottle_dir(&options.id);
        if dir.exists() {
            return Err(Error::BottleExists(options.id.clone()));
        }
        let engines = EngineManager::new(self.dirs);
        let engine = engines
            .get(&options.engine)?
            .ok_or_else(|| Error::EngineNotInstalled(options.engine.clone()))?;
        if engine.translation == Translation::External {
            return Err(Error::InvalidInput(format!(
                "{} is an external engine: create the bottle in CrossOver, then run \
                 `cross crossover import <bottle>`",
                options.engine
            )));
        }
        engines.check_runnable(&engine)?;

        fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let wine = engines.wine_path(&engine);
        let result = self.create_in(options, &wine);
        if result.is_err() {
            // A half-initialised prefix may still have a live wineserver writing into it.
            let _ = Runtime::new(self.dirs, &options.id, &wine).stop(StopMode::Force);
            let _ = fs::remove_dir_all(&dir);
        }
        result
    }

    fn create_in(&self, options: &CreateOptions, wine: &Path) -> Result<Bottle> {
        let prefix = self.dirs.bottle_prefix(&options.id);
        fs::create_dir_all(&prefix).map_err(|e| Error::io(&prefix, e))?;
        let runtime = Runtime::new(self.dirs, &options.id, wine);

        tracing::info!(bottle = %options.id, engine = %options.engine, "initialising prefix");
        let mut boot = runtime.wine_command();
        // Do not pop up Mono/Gecko download dialogs during first start.
        boot.env("WINEDLLOVERRIDES", "mscoree,mshtml=")
            .args(["wineboot", "--init"]);
        run_checked(boot, "wineboot --init", WINEBOOT_TIMEOUT)?;

        runtime.set_windows_version(options.windows)?;
        // Wine writes the registry files when its wineserver exits, which is also
        // when the prefix becomes visible on disk: wait for that before checking.
        let mut wait = runtime.wineserver_command();
        wait.arg("-w");
        run_captured(wait, "wineserver -w", WINESERVER_WAIT_TIMEOUT)?;
        if !prefix.join("system.reg").is_file() {
            return Err(Error::Wine {
                what: "wineboot --init".into(),
                message: "finished without creating a Wine prefix".into(),
            });
        }

        let mut bottle = Bottle::new(&options.id, options.name.as_deref().unwrap_or(&options.id))?;
        bottle.engine = Some(options.engine.clone());
        bottle.windows_version = options.windows;
        bottle.created_unix = now_unix();
        BottleStore::new(self.dirs).save(&bottle)?;
        Ok(bottle)
    }

    /// Delete a bottle and everything in it, stopping its processes first.
    /// For a bottle imported from CrossOver only the link is removed
    /// (`remove_dir_all` does not follow symlinks).
    pub fn delete(&self, id: &str) -> Result<()> {
        self.get(id)?;
        if let Ok((_, runtime)) = Runtime::open(self.dirs, id) {
            runtime.stop(StopMode::Force)?;
        }
        let dir = self.dirs.bottle_dir(id);
        fs::remove_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        tracing::info!(bottle = id, "bottle deleted");
        Ok(())
    }

    /// Copy a bottle (APFS clone when possible, so it is instant and uses no extra space).
    pub fn clone_bottle(&self, source: &str, target: &str, name: Option<&str>) -> Result<Bottle> {
        let mut bottle = self.get(source)?;
        validate_bottle_id(target)?;
        if crate::crossover::is_linked(self.dirs, source) {
            return Err(Error::InvalidInput(format!(
                "bottle {source:?} is linked to a CrossOver bottle; duplicate it in CrossOver instead"
            )));
        }
        self.ensure_stopped(source)?;
        let dest = self.dirs.bottle_dir(target);
        if dest.exists() {
            return Err(Error::BottleExists(target.to_owned()));
        }

        let src = self.dirs.bottle_dir(source);
        if let Err(e) = copy_tree(&src, &dest) {
            let _ = fs::remove_dir_all(&dest);
            return Err(e);
        }
        let _ = fs::remove_dir_all(self.dirs.bottle_logs_dir(target));

        bottle.id = target.to_owned();
        bottle.name = name.map_or_else(|| format!("{} (copy)", bottle.name), str::to_owned);
        bottle.created_unix = now_unix();
        if let Err(e) = BottleStore::new(self.dirs).save(&bottle) {
            let _ = fs::remove_dir_all(&dest);
            return Err(e.into());
        }
        Ok(bottle)
    }

    /// Change a bottle's id (its directory name). The display name follows
    /// unless it was customised.
    pub fn rename(&self, id: &str, new_id: &str) -> Result<Bottle> {
        let mut bottle = self.get(id)?;
        validate_bottle_id(new_id)?;
        self.ensure_stopped(id)?;
        let to = self.dirs.bottle_dir(new_id);
        if to.exists() {
            return Err(Error::BottleExists(new_id.to_owned()));
        }
        let from = self.dirs.bottle_dir(id);
        fs::rename(&from, &to).map_err(|e| Error::io(&from, e))?;

        if bottle.name == bottle.id {
            bottle.name = new_id.to_owned();
        }
        bottle.id = new_id.to_owned();
        if let Err(e) = BottleStore::new(self.dirs).save(&bottle) {
            let _ = fs::rename(&to, &from);
            return Err(e.into());
        }
        Ok(bottle)
    }

    /// Refuse to touch a bottle that is running. A wineserver lingers a few
    /// seconds after the last program exits, so wait briefly before giving up.
    fn ensure_stopped(&self, id: &str) -> Result<()> {
        let prefix = self.dirs.bottle_prefix(id);
        let deadline = Instant::now() + IDLE_GRACE;
        loop {
            let count = running_wineservers(&prefix)?.len();
            if count == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error::BottleRunning {
                    bottle: id.to_owned(),
                    count,
                });
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

/// `cp -cR` (clonefile on APFS), falling back to a plain recursive copy.
fn copy_tree(src: &Path, dest: &Path) -> Result<()> {
    let attempt = |flags: &str| {
        let mut cmd = Command::new("cp");
        cmd.arg(flags).arg(src).arg(dest);
        run_captured(cmd, "cp", COPY_TIMEOUT)
    };
    let cloned = attempt("-cR")?;
    if cloned.status.success() {
        return Ok(());
    }
    // Cross-volume or non-APFS: clean up a partial result and copy normally.
    let _ = fs::remove_dir_all(dest);
    let copied = attempt("-R")?;
    if copied.status.success() {
        Ok(())
    } else {
        Err(Error::Wine {
            what: "copy bottle".into(),
            message: copied.message(),
        })
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use cross_core::model::Translation;
    use cross_engine::manager::{InstalledEngine, ENGINE_FILE};

    use super::*;
    use crate::runtime::test_support::{fake_wine, FakeWine};

    struct Fixture {
        _tmp: tempfile::TempDir,
        dirs: AppDirs,
        fake: FakeWine,
    }

    /// An "installed" engine whose wine is the recording fake.
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        // Build the fake inside the engine directory so that `wine_bin` resolves to it.
        let engine_dir = dirs.engine_dir("wine-fake");
        let fake = fake_wine(&engine_dir);
        let engine = InstalledEngine {
            id: "wine-fake".into(),
            name: "Fake".into(),
            wine_version: "0".into(),
            translation: Translation::Rosetta,
            wine_bin: "fake-engine/bin/wine".into(),
            backends: vec![],
            components: vec![],
            source_sha256: "0".repeat(64),
            installed_unix: 0,
        };
        fs::write(
            engine_dir.join(ENGINE_FILE),
            toml::to_string(&engine).unwrap(),
        )
        .unwrap();
        Fixture {
            _tmp: tmp,
            dirs,
            fake,
        }
    }

    fn options(id: &str) -> CreateOptions {
        CreateOptions {
            id: id.into(),
            name: None,
            engine: "wine-fake".into(),
            windows: WindowsVersion::Win10,
        }
    }

    #[test]
    fn create_initialises_the_prefix_and_records_the_bottle() {
        let fx = fixture();
        let manager = BottleManager::new(&fx.dirs);
        let bottle = manager.create(&options("demo")).unwrap();

        assert_eq!(bottle.engine.as_deref(), Some("wine-fake"));
        assert_eq!(bottle.windows_version, WindowsVersion::Win10);
        assert!(bottle.created_unix > 0);
        assert!(fx.dirs.bottle_prefix("demo").join("system.reg").is_file());
        assert_eq!(manager.get("demo").unwrap(), bottle);
        assert_eq!(manager.list().unwrap(), vec![bottle]);

        let calls = fx.fake.calls();
        assert!(calls.contains("ARGS:wineboot --init"), "{calls}");
        assert!(calls.contains("DLL=mscoree,mshtml="), "{calls}");
        assert!(
            calls.contains("ARGS:reg add HKCU\\Software\\Wine /v Version /t REG_SZ /d win10 /f"),
            "{calls}"
        );
        assert!(calls.contains(&format!(
            "PREFIX:{}",
            fx.dirs.bottle_prefix("demo").display()
        )));
        assert!(calls.contains("SERVER:-w"), "{calls}");
    }

    #[test]
    fn create_rejects_duplicates_bad_ids_and_missing_engines() {
        let fx = fixture();
        let manager = BottleManager::new(&fx.dirs);
        manager.create(&options("demo")).unwrap();
        assert!(matches!(
            manager.create(&options("demo")).unwrap_err(),
            Error::BottleExists(_)
        ));
        assert!(matches!(
            manager.create(&options("../x")).unwrap_err(),
            Error::Core(_)
        ));

        let mut missing = options("other");
        missing.engine = "nope".into();
        assert!(matches!(
            manager.create(&missing).unwrap_err(),
            Error::EngineNotInstalled(_)
        ));
        assert!(!fx.dirs.bottle_dir("other").exists());
    }

    #[test]
    fn failed_create_leaves_nothing_behind() {
        let fx = fixture();
        // Replace the fake wine with one that fails.
        fs::write(&fx.fake.wine, "#!/bin/sh\necho boom >&2\nexit 1\n").unwrap();
        let manager = BottleManager::new(&fx.dirs);
        let err = manager.create(&options("demo")).unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
        assert!(!fx.dirs.bottle_dir("demo").exists());
    }

    #[test]
    fn create_fails_when_wineboot_makes_no_prefix() {
        let fx = fixture();
        fs::write(&fx.fake.wine, "#!/bin/sh\nexit 0\n").unwrap();
        let manager = BottleManager::new(&fx.dirs);
        let err = manager.create(&options("demo")).unwrap_err();
        assert!(err.to_string().contains("without creating"), "{err}");
        assert!(!fx.dirs.bottle_dir("demo").exists());
    }

    #[test]
    fn engine_resolution_order() {
        let fx = fixture();
        let manager = BottleManager::new(&fx.dirs);
        let mut config = Config::default();
        assert_eq!(manager.resolve_engine(None, &config).unwrap(), "wine-fake");
        assert_eq!(manager.resolve_engine(Some("x"), &config).unwrap(), "x");
        config.default_engine = Some("cfg".into());
        assert_eq!(manager.resolve_engine(None, &config).unwrap(), "cfg");

        let empty = AppDirs::with_root(fx.dirs.root().join("empty"));
        let err = BottleManager::new(&empty)
            .resolve_engine(None, &Config::default())
            .unwrap_err();
        assert!(matches!(err, Error::NoEngineChosen { .. }), "{err}");
    }

    #[test]
    fn clone_copies_the_prefix_under_a_new_identity() {
        let fx = fixture();
        let manager = BottleManager::new(&fx.dirs);
        manager.create(&options("demo")).unwrap();
        fs::write(fx.dirs.bottle_prefix("demo").join("marker"), "x").unwrap();
        fs::create_dir_all(fx.dirs.bottle_logs_dir("demo")).unwrap();
        fs::write(fx.dirs.bottle_logs_dir("demo").join("cross.log"), "old").unwrap();

        let copy = manager.clone_bottle("demo", "demo2", None).unwrap();
        assert_eq!(copy.id, "demo2");
        assert_eq!(copy.name, "demo (copy)");
        assert_eq!(copy.engine.as_deref(), Some("wine-fake"));
        assert!(fx.dirs.bottle_prefix("demo2").join("marker").is_file());
        assert!(!fx.dirs.bottle_logs_dir("demo2").exists());
        assert_eq!(manager.get("demo").unwrap().id, "demo");
        assert_eq!(manager.list().unwrap().len(), 2);

        assert!(matches!(
            manager.clone_bottle("demo", "demo2", None).unwrap_err(),
            Error::BottleExists(_)
        ));
        assert!(matches!(
            manager.clone_bottle("nope", "x", None).unwrap_err(),
            Error::BottleNotFound(_)
        ));
        let named = manager
            .clone_bottle("demo", "demo3", Some("Third"))
            .unwrap();
        assert_eq!(named.name, "Third");
    }

    #[test]
    fn rename_moves_the_directory_and_follows_the_default_name() {
        let fx = fixture();
        let manager = BottleManager::new(&fx.dirs);
        manager.create(&options("demo")).unwrap();
        manager.create(&options("keep")).unwrap();

        let renamed = manager.rename("demo", "steam").unwrap();
        assert_eq!(
            (renamed.id.as_str(), renamed.name.as_str()),
            ("steam", "steam")
        );
        assert!(!fx.dirs.bottle_dir("demo").exists());
        assert!(fx.dirs.bottle_prefix("steam").join("system.reg").is_file());
        assert!(matches!(
            manager.get("demo").unwrap_err(),
            Error::BottleNotFound(_)
        ));

        assert!(matches!(
            manager.rename("steam", "keep").unwrap_err(),
            Error::BottleExists(_)
        ));
        assert!(matches!(
            manager.rename("steam", "Bad Id").unwrap_err(),
            Error::Core(_)
        ));
        assert_eq!(manager.get("steam").unwrap().id, "steam");
    }

    #[test]
    fn rename_keeps_a_customised_display_name() {
        let fx = fixture();
        let manager = BottleManager::new(&fx.dirs);
        let mut opts = options("demo");
        opts.name = Some("My Games".into());
        manager.create(&opts).unwrap();
        assert_eq!(manager.rename("demo", "games").unwrap().name, "My Games");
    }

    #[test]
    fn delete_removes_the_bottle() {
        let fx = fixture();
        let manager = BottleManager::new(&fx.dirs);
        manager.create(&options("demo")).unwrap();
        manager.delete("demo").unwrap();
        assert!(!fx.dirs.bottle_dir("demo").exists());
        assert!(manager.list().unwrap().is_empty());
        assert!(matches!(
            manager.delete("demo").unwrap_err(),
            Error::BottleNotFound(_)
        ));
    }
}
