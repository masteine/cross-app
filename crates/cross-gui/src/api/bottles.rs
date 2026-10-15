//! Bottle page: summary, settings, dependencies, processes, start/stop.

use cross_bottle::launch::Tool;
use cross_bottle::manager::{BottleManager, CreateOptions};
use cross_bottle::processes::{running_wineservers, StopMode, WinProcess};
use cross_bottle::Runtime;
use cross_core::config::{BottleStore, Config};
use cross_core::model::{Bottle, GraphicsBackend, WindowsVersion};
use cross_core::paths::AppDirs;
use cross_deps::install::{InstallOptions, Installer, Status};
use cross_engine::manager::EngineManager;
use cross_engine::Progress;
use serde::{Deserialize, Serialize};

use crate::api::apps::{self, AppView};
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BottleSummary {
    pub id: String,
    pub name: String,
    pub engine: Option<String>,
    pub windows: WindowsVersion,
    pub backend: GraphicsBackend,
    pub running: bool,
    pub steam_installed: bool,
    pub apps: usize,
    pub created_unix: u64,
}

fn summary(dirs: &AppDirs, bottle: &Bottle) -> BottleSummary {
    let prefix = dirs.bottle_prefix(&bottle.id);
    BottleSummary {
        id: bottle.id.clone(),
        name: bottle.name.clone(),
        engine: bottle.engine.clone(),
        windows: bottle.windows_version,
        backend: bottle.backend,
        // `ps` failing is not worth an error on a list page.
        running: running_wineservers(&prefix).is_ok_and(|p| !p.is_empty()),
        steam_installed: cross_steam::install::is_installed(&prefix),
        apps: bottle.apps.len(),
        created_unix: bottle.created_unix,
    }
}

pub fn list(dirs: &AppDirs) -> Result<Vec<BottleSummary>> {
    Ok(BottleManager::new(dirs)
        .list()?
        .iter()
        .map(|b| summary(dirs, b))
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DepView {
    pub verb: String,
    pub title: Option<String>,
    pub installed: bool,
    /// `catalog` or `winetricks`, for installed verbs.
    pub source: Option<cross_core::model::DepSource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BottleDetail {
    #[serde(flatten)]
    pub summary: BottleSummary,
    pub apps: Vec<AppView>,
    pub deps: Vec<DepView>,
    pub winetricks: Option<String>,
    /// Installed engines this bottle can be switched to.
    pub engines: Vec<String>,
    pub logs_dir: String,
    pub prefix: String,
}

pub fn detail(dirs: &AppDirs, id: &str) -> Result<BottleDetail> {
    let bottle = BottleManager::new(dirs).get(id)?;
    let installer = Installer::new(dirs)?;
    let mut deps: Vec<DepView> = installer
        .catalog()
        .verbs
        .iter()
        .map(|(verb, v)| DepView {
            verb: verb.clone(),
            title: Some(v.title.clone()),
            installed: bottle.deps.contains_key(verb),
            source: bottle.deps.get(verb).map(|r| r.source),
        })
        .collect();
    // Verbs installed through winetricks (or from an older catalog).
    for (verb, record) in &bottle.deps {
        if installer.catalog().get(verb).is_none() {
            deps.push(DepView {
                verb: verb.clone(),
                title: None,
                installed: true,
                source: Some(record.source),
            });
        }
    }
    let engines = EngineManager::new(dirs)
        .list()?
        .into_iter()
        .map(|e| e.id)
        .collect();
    Ok(BottleDetail {
        summary: summary(dirs, &bottle),
        apps: apps::views(&bottle),
        deps,
        winetricks: installer.winetricks_path().map(|p| p.display().to_string()),
        engines,
        logs_dir: dirs.bottle_logs_dir(id).display().to_string(),
        prefix: dirs.bottle_prefix(id).display().to_string(),
    })
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewBottle {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// `None`: the configured default, else the only installed engine.
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub windows: WindowsVersion,
}

/// Create a bottle (runs `wineboot --init`; takes about half a minute).
pub fn create(dirs: &AppDirs, new: NewBottle) -> Result<BottleSummary> {
    let config = Config::load(dirs)?;
    let manager = BottleManager::new(dirs);
    let engine = manager.resolve_engine(new.engine.as_deref(), &config)?;
    let name = new.name.filter(|n| !n.trim().is_empty());
    tracing::info!(bottle = %new.id, %engine, "creating bottle from the GUI");
    let bottle = manager.create(&CreateOptions {
        id: new.id,
        name,
        engine,
        windows: new.windows,
    })?;
    Ok(summary(dirs, &bottle))
}

pub fn delete(dirs: &AppDirs, id: &str) -> Result<()> {
    Ok(BottleManager::new(dirs).delete(id)?)
}

/// Change the Windows version in the registry and keep `bottle.toml` in step.
pub fn set_windows(dirs: &AppDirs, id: &str, version: WindowsVersion) -> Result<()> {
    let (mut bottle, runtime) = Runtime::open(dirs, id)?;
    runtime.set_windows_version(version)?;
    bottle.windows_version = version;
    BottleStore::new(dirs).save(&bottle)?;
    tracing::info!(
        bottle = id,
        windows = version.wine_name(),
        "windows version changed"
    );
    Ok(())
}

/// Point a stopped bottle at another installed engine.
pub fn set_engine(dirs: &AppDirs, id: &str, engine: &str) -> Result<()> {
    let mut bottle = BottleManager::new(dirs).get(id)?;
    if EngineManager::new(dirs).get(engine)?.is_none() {
        return Err(cross_bottle::Error::EngineNotInstalled(engine.to_owned()).into());
    }
    let running = running_wineservers(&dirs.bottle_prefix(id))?;
    if !running.is_empty() {
        return Err(cross_bottle::Error::BottleRunning {
            bottle: id.to_owned(),
            count: running.len(),
        }
        .into());
    }
    bottle.engine = Some(engine.to_owned());
    BottleStore::new(dirs).save(&bottle)?;
    tracing::info!(bottle = id, engine, "bottle engine changed");
    Ok(())
}

pub fn processes(dirs: &AppDirs, id: &str) -> Result<Vec<WinProcess>> {
    let (_, runtime) = Runtime::open(dirs, id)?;
    Ok(runtime.processes()?)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StopResult {
    pub was_running: bool,
    pub remaining: usize,
}

pub fn stop(dirs: &AppDirs, id: &str, force: bool) -> Result<StopResult> {
    let (_, runtime) = Runtime::open(dirs, id)?;
    let mode = if force {
        StopMode::Force
    } else {
        StopMode::Graceful
    };
    let outcome = runtime.stop(mode)?;
    Ok(StopResult {
        was_running: outcome.was_running,
        remaining: outcome.remaining,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DepOutcome {
    pub verb: String,
    /// `installed`, `installed-winetricks` or `already-installed`.
    pub status: &'static str,
}

/// Install verbs. The system winetricks is used only when `allow_winetricks`
/// is set; the page asks before enabling it, like the CLI warns.
pub fn install_deps(
    dirs: &AppDirs,
    id: &str,
    verbs: &[String],
    force: bool,
    allow_winetricks: bool,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<Vec<DepOutcome>> {
    if verbs.is_empty() {
        return Err(Error::InvalidInput("no dependency selected".into()));
    }
    let installer = Installer::new(dirs)?;
    tracing::info!(bottle = id, ?verbs, "installing dependencies from the GUI");
    let outcomes = installer.install(
        id,
        verbs,
        InstallOptions {
            force,
            allow_winetricks,
        },
        progress,
    )?;
    Ok(outcomes
        .into_iter()
        .map(|o| DepOutcome {
            verb: o.verb,
            status: match o.status {
                Status::Installed(cross_core::model::DepSource::Catalog) => "installed",
                Status::Installed(cross_core::model::DepSource::Winetricks) => {
                    "installed-winetricks"
                }
                Status::AlreadyInstalled => "already-installed",
            },
        })
        .collect())
}

pub fn parse_tool(name: &str) -> Result<Tool> {
    match name {
        "winecfg" => Ok(Tool::Winecfg),
        "regedit" => Ok(Tool::Regedit),
        "cmd" => Ok(Tool::Cmd),
        "taskmgr" => Ok(Tool::Taskmgr),
        other => Err(Error::InvalidInput(format!("unknown tool {other:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use cross_core::model::{DepRecord, DepSource};

    use super::*;
    use crate::api::test_support::{fake_bottle, fake_engine};

    #[test]
    fn list_and_detail_reflect_bottle_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        assert!(list(&dirs).unwrap().is_empty());

        let mut bottle = fake_bottle(&dirs, "games", "wine-a");
        bottle.deps.insert(
            "dxvk".into(),
            DepRecord {
                installed_unix: 1,
                source: DepSource::Catalog,
            },
        );
        bottle.deps.insert(
            "corefonts".into(),
            DepRecord {
                installed_unix: 2,
                source: DepSource::Winetricks,
            },
        );
        BottleStore::new(&dirs).save(&bottle).unwrap();

        let rows = list(&dirs).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].engine.as_deref(), Some("wine-a"));
        assert!(!rows[0].running && !rows[0].steam_installed);

        let detail = detail(&dirs, "games").unwrap();
        let dxvk = detail.deps.iter().find(|d| d.verb == "dxvk").unwrap();
        assert!(dxvk.installed && dxvk.title.is_some());
        let vcrun = detail.deps.iter().find(|d| d.verb == "vcrun2022").unwrap();
        assert!(!vcrun.installed);
        let fonts = detail.deps.iter().find(|d| d.verb == "corefonts").unwrap();
        assert_eq!(fonts.source, Some(DepSource::Winetricks));
        assert_eq!(detail.engines, ["wine-a"]);

        let json = serde_json::to_value(&detail).unwrap();
        assert_eq!(json["id"], "games", "summary is flattened");
        assert_eq!(json["windows"], "win11");
    }

    #[test]
    fn missing_bottles_and_bad_ids_are_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let err = detail(&dirs, "nope").unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
        assert!(detail(&dirs, "../etc").is_err());
    }

    #[test]
    fn engine_switch_requires_an_installed_engine() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        fake_bottle(&dirs, "demo", "wine-a");
        let err = set_engine(&dirs, "demo", "wine-b").unwrap_err();
        assert!(err.to_string().contains("not installed"), "{err}");
        fake_engine(&dirs, "wine-b");
        set_engine(&dirs, "demo", "wine-b").unwrap();
        assert_eq!(
            BottleStore::new(&dirs)
                .load("demo")
                .unwrap()
                .engine
                .as_deref(),
            Some("wine-b")
        );
    }

    #[test]
    fn create_reports_when_no_engine_can_be_chosen() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let err = create(
            &dirs,
            NewBottle {
                id: "first".into(),
                name: None,
                engine: None,
                windows: WindowsVersion::Win10,
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("no engine chosen"), "{err}");
        // The page sends JSON; only `id` is required.
        let new: NewBottle = serde_json::from_str(r#"{"id":"x"}"#).unwrap();
        assert_eq!(new.windows, WindowsVersion::Win11);
    }

    #[test]
    fn stopped_bottle_has_no_processes_and_stop_is_a_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        fake_bottle(&dirs, "demo", "wine-a");
        assert!(processes(&dirs, "demo").unwrap().is_empty());
        let result = stop(&dirs, "demo", false).unwrap();
        assert!(!result.was_running);
    }

    #[test]
    fn deps_need_a_selection_and_tools_a_known_name() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        fake_bottle(&dirs, "demo", "wine-a");
        assert!(install_deps(&dirs, "demo", &[], false, false, &mut |_| {}).is_err());
        assert_eq!(parse_tool("winecfg").unwrap(), Tool::Winecfg);
        assert!(parse_tool("rm").is_err());
    }
}
