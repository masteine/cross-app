//! A bottle bound to the Wine binary that runs it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use cross_core::config::BottleStore;
use cross_core::model::Bottle;
use cross_core::paths::AppDirs;
use cross_engine::manager::EngineManager;

use crate::{Error, Result};

/// Everything needed to spawn Wine processes inside one bottle.
#[derive(Debug, Clone)]
pub struct Runtime {
    bottle_id: String,
    prefix: PathBuf,
    wine: PathBuf,
    wineserver: PathBuf,
}

impl Runtime {
    /// Load a bottle and the engine it uses.
    pub fn open(dirs: &AppDirs, bottle_id: &str) -> Result<(Bottle, Runtime)> {
        let bottle = BottleStore::new(dirs)
            .load(bottle_id)
            .map_err(|e| match e {
                cross_core::Error::Io { ref source, .. }
                    if source.kind() == std::io::ErrorKind::NotFound =>
                {
                    Error::BottleNotFound(bottle_id.to_owned())
                }
                other => other.into(),
            })?;
        let engine_id = bottle
            .engine
            .clone()
            .ok_or_else(|| Error::BottleHasNoEngine {
                bottle: bottle.id.clone(),
            })?;
        let engines = EngineManager::new(dirs);
        let engine = engines
            .get(&engine_id)?
            .ok_or(Error::EngineNotInstalled(engine_id))?;
        let runtime = Runtime::new(dirs, bottle_id, &engines.wine_path(&engine));
        Ok((bottle, runtime))
    }

    /// Bind a bottle to an explicit `wine` executable.
    pub fn new(dirs: &AppDirs, bottle_id: &str, wine: &Path) -> Self {
        Self {
            bottle_id: bottle_id.to_owned(),
            prefix: dirs.bottle_prefix(bottle_id),
            wine: wine.to_owned(),
            wineserver: wine.with_file_name("wineserver"),
        }
    }

    pub fn bottle_id(&self) -> &str {
        &self.bottle_id
    }

    pub fn prefix(&self) -> &Path {
        &self.prefix
    }

    /// The `wine` executable this bottle runs with.
    pub fn wine_path(&self) -> &Path {
        &self.wine
    }

    /// Any program (e.g. `winetricks`) with this bottle's Wine environment:
    /// `WINEPREFIX`/`WINEARCH`/`WINEDEBUG`, plus `WINE` and `WINESERVER`, and the
    /// engine's `bin` directory first on `PATH`.
    pub fn command_for(&self, program: &Path) -> Command {
        let mut cmd = self.command(program);
        cmd.env("WINE", &self.wine)
            .env("WINESERVER", &self.wineserver);
        if let Some(bin) = self.wine.parent() {
            let mut paths = vec![bin.to_owned()];
            if let Some(existing) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&existing));
            }
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("PATH", joined);
            }
        }
        cmd
    }

    /// `wine` with the prefix environment set.
    pub fn wine_command(&self) -> Command {
        self.command(&self.wine)
    }

    /// `wineserver` for this prefix.
    pub fn wineserver_command(&self) -> Command {
        self.command(&self.wineserver)
    }

    fn command(&self, program: &Path) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("WINEPREFIX", &self.prefix).env("WINEARCH", "win64");
        // Wine is chatty; keep its debug channels off unless the user turned them on.
        let debug: OsString = std::env::var_os("WINEDEBUG").unwrap_or_else(|| "-all".into());
        cmd.env("WINEDEBUG", debug);
        cmd
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A fake `wine` + `wineserver` pair that records invocations.
    ///
    /// `wine <args>` appends `ARGS:<args>` and `PREFIX:<WINEPREFIX>` to `calls.log`
    /// next to the script; `wine wineboot ...` also creates `system.reg` in the
    /// prefix, and `wine reg ...` is answered from `reg.db`.
    pub struct FakeWine {
        pub wine: PathBuf,
        pub log: PathBuf,
    }

    pub fn fake_wine(dir: &Path) -> FakeWine {
        let bin = dir.join("fake-engine/bin");
        fs::create_dir_all(&bin).unwrap();
        let wine = bin.join("wine");
        let log = bin.join("calls.log");
        let script = r#"#!/bin/sh
here="$(dirname "$0")"
echo "ARGS:$*" >> "$here/calls.log"
echo "PREFIX:$WINEPREFIX" >> "$here/calls.log"
echo "ENV:MSYNC=$WINEMSYNC HUD=$MTL_HUD_ENABLED DLL=$WINEDLLOVERRIDES" >> "$here/calls.log"
echo "CWD:$(pwd)" >> "$here/calls.log"
case "$1" in
  wineboot)
    if [ "$2" = "--init" ]; then
      mkdir -p "$WINEPREFIX"
      echo "WINE REGISTRY Version 2" > "$WINEPREFIX/system.reg"
    fi ;;
  reg)
    case "$2" in
      add)
        # reg add KEY /v NAME /t TYPE /d DATA /f
        echo "$3|$5|$7|$9" >> "$here/reg.db" ;;
      query)
        key="$3"; name="$5"
        line="$(grep -F "$key|$name|" "$here/reg.db" 2>/dev/null | tail -1)"
        if [ -z "$line" ]; then
          echo "reg: Unable to find the specified registry value"
          exit 1
        fi
        type="$(echo "$line" | cut -d'|' -f3)"; data="$(echo "$line" | cut -d'|' -f4)"
        printf '%s\n    %s    %s    %s\n' "$key" "$name" "$type" "$data" ;;
      delete)
        grep -vF "$3|$5|" "$here/reg.db" > "$here/reg.db.new" 2>/dev/null
        mv "$here/reg.db.new" "$here/reg.db" ;;
    esac ;;
esac
exit 0
"#;
        fs::write(&wine, script).unwrap();
        fs::set_permissions(&wine, fs::Permissions::from_mode(0o755)).unwrap();
        let wineserver = bin.join("wineserver");
        fs::write(
            &wineserver,
            "#!/bin/sh\necho \"SERVER:$*\" >> \"$(dirname \"$0\")/calls.log\"\nexit 0\n",
        )
        .unwrap();
        fs::set_permissions(&wineserver, fs::Permissions::from_mode(0o755)).unwrap();
        FakeWine { wine, log }
    }

    impl FakeWine {
        pub fn calls(&self) -> String {
            fs::read_to_string(&self.log).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::fake_wine;
    use super::*;

    #[test]
    fn commands_carry_the_prefix_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let fake = fake_wine(tmp.path());
        let rt = Runtime::new(&dirs, "demo", &fake.wine);

        let cmd = rt.wine_command();
        assert_eq!(cmd.get_program(), fake.wine.as_os_str());
        let env: std::collections::HashMap<_, _> = cmd.get_envs().collect();
        assert_eq!(
            env[std::ffi::OsStr::new("WINEPREFIX")].unwrap(),
            dirs.bottle_prefix("demo").as_os_str()
        );
        assert_eq!(env[std::ffi::OsStr::new("WINEARCH")].unwrap(), "win64");

        assert_eq!(
            rt.wineserver_command().get_program(),
            fake.wine.with_file_name("wineserver").as_os_str()
        );
    }

    #[test]
    fn open_reports_missing_bottle_engine_and_install() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        assert!(matches!(
            Runtime::open(&dirs, "nope").unwrap_err(),
            Error::BottleNotFound(_)
        ));

        let store = BottleStore::new(&dirs);
        store.save(&Bottle::new("bare", "Bare").unwrap()).unwrap();
        assert!(matches!(
            Runtime::open(&dirs, "bare").unwrap_err(),
            Error::BottleHasNoEngine { .. }
        ));

        let mut bottle = Bottle::new("orphan", "Orphan").unwrap();
        bottle.engine = Some("wine-gone".into());
        store.save(&bottle).unwrap();
        assert!(matches!(
            Runtime::open(&dirs, "orphan").unwrap_err(),
            Error::EngineNotInstalled(_)
        ));
    }
}
