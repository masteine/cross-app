//! Engine settings page: catalog and installed engines, GPTK, default engine.

use std::path::Path;

use cross_core::config::Config;
use cross_core::model::Translation;
use cross_core::paths::AppDirs;
use cross_core::system::{self, GptkInfo};
use cross_engine::catalog::Catalog;
use cross_engine::manager::{EngineManager, InstallOptions, InstalledEngine};
use cross_engine::{gptk, Progress};
use serde::Serialize;

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EngineRow {
    pub id: String,
    pub name: String,
    pub wine_version: String,
    pub translation: Translation,
    pub supports_dxvk: bool,
    pub installed: bool,
    pub dxvk_installed: bool,
    /// Installed from a catalog that no longer lists it.
    pub in_catalog: bool,
    /// Download size of the engine archive, when the catalog knows it.
    pub size: Option<u64>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnginesView {
    pub engines: Vec<EngineRow>,
    pub gptk: Option<GptkInfo>,
    pub rosetta_installed: bool,
}

pub fn list(dirs: &AppDirs) -> Result<EnginesView> {
    let config = Config::load(dirs)?;
    let catalog = Catalog::load(dirs)?;
    let installed = EngineManager::new(dirs).list()?;
    let is_default = |id: &str| config.default_engine.as_deref() == Some(id);
    let local = |id: &str| installed.iter().find(|e| e.id == id);

    let mut engines: Vec<EngineRow> = catalog
        .engines
        .iter()
        .map(|e| EngineRow {
            id: e.id.clone(),
            name: e.name.clone(),
            wine_version: e.wine_version.clone(),
            translation: e.translation,
            supports_dxvk: e.supports_dxvk(),
            installed: local(&e.id).is_some(),
            dxvk_installed: local(&e.id).is_some_and(InstalledEngine::has_dxvk),
            in_catalog: true,
            // External engines (CrossOver) have no download.
            size: e.archive.as_ref().and_then(|a| a.size),
            is_default: is_default(&e.id),
        })
        .collect();
    for e in installed.iter().filter(|e| catalog.engine(&e.id).is_none()) {
        engines.push(EngineRow {
            id: e.id.clone(),
            name: e.name.clone(),
            wine_version: e.wine_version.clone(),
            translation: e.translation,
            supports_dxvk: false,
            installed: true,
            dxvk_installed: e.has_dxvk(),
            in_catalog: false,
            size: None,
            is_default: is_default(&e.id),
        });
    }
    Ok(EnginesView {
        engines,
        gptk: GptkInfo::probe(&config.gptk_dir(dirs)),
        rosetta_installed: system::rosetta_installed(),
    })
}

/// Install an engine, or add DXVK + MoltenVK to one that is already installed
/// (the same rule as `cross engine install <id> --dxvk`).
pub fn install(
    dirs: &AppDirs,
    id: &str,
    dxvk: bool,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<EngineRow> {
    let catalog = Catalog::load(dirs)?;
    let manager = EngineManager::new(dirs);
    tracing::info!(engine = id, dxvk, "installing engine from the GUI");
    let installed = match manager.get(id)? {
        Some(existing) if dxvk && !existing.has_dxvk() => {
            manager.add_dxvk(&catalog, id, progress)?
        }
        Some(_) => return Err(cross_engine::Error::AlreadyInstalled(id.to_owned()).into()),
        None => manager.install(&catalog, id, InstallOptions { with_dxvk: dxvk }, progress)?,
    };
    // The first engine becomes the default, so the wizard's next step needs no choice.
    let mut config = Config::load(dirs)?;
    if config.default_engine.is_none() {
        config.default_engine = Some(installed.id.clone());
        config.save(dirs)?;
    }
    let row = list(dirs)?
        .engines
        .into_iter()
        .find(|e| e.id == installed.id)
        .ok_or_else(|| Error::Internal(format!("{} vanished after install", installed.id)))?;
    Ok(row)
}

pub fn remove(dirs: &AppDirs, id: &str) -> Result<()> {
    EngineManager::new(dirs).remove(id)?;
    let mut config = Config::load(dirs)?;
    if config.default_engine.as_deref() == Some(id) {
        config.default_engine = None;
        config.save(dirs)?;
    }
    Ok(())
}

pub fn set_default(dirs: &AppDirs, id: &str) -> Result<()> {
    if EngineManager::new(dirs).get(id)?.is_none() {
        return Err(cross_engine::Error::NotInstalled(id.to_owned()).into());
    }
    let mut config = Config::load(dirs)?;
    config.default_engine = Some(id.to_owned());
    config.save(dirs)?;
    tracing::info!(engine = id, "default engine changed");
    Ok(())
}

pub fn import_gptk(dirs: &AppDirs, source: &Path, version: Option<&str>) -> Result<GptkInfo> {
    let config = Config::load(dirs)?;
    tracing::info!(source = %source.display(), "importing GPTK from the GUI");
    let imported = gptk::import(source, &config.gptk_dir(dirs), version)?;
    Ok(GptkInfo {
        dir: imported.dir,
        version: imported.version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::fake_engine as fake_installed;

    #[test]
    fn lists_catalog_and_orphaned_engines() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let view = list(&dirs).unwrap();
        assert!(!view.engines.is_empty(), "built-in catalog has engines");
        assert!(view.engines.iter().all(|e| e.in_catalog && !e.installed));

        fake_installed(&dirs, "wine-local");
        set_default(&dirs, "wine-local").unwrap();
        let view = list(&dirs).unwrap();
        let local = view.engines.iter().find(|e| e.id == "wine-local").unwrap();
        assert!(local.installed && !local.in_catalog && local.is_default);
    }

    #[test]
    fn default_engine_must_be_installed_and_is_cleared_on_remove() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        assert!(set_default(&dirs, "wine-none").is_err());

        fake_installed(&dirs, "wine-a");
        set_default(&dirs, "wine-a").unwrap();
        assert_eq!(
            Config::load(&dirs).unwrap().default_engine.as_deref(),
            Some("wine-a")
        );
        remove(&dirs, "wine-a").unwrap();
        assert_eq!(Config::load(&dirs).unwrap().default_engine, None);
        assert!(!dirs.engine_dir("wine-a").exists());
    }

    #[test]
    fn reinstalling_without_dxvk_is_refused_before_any_download() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let id = Catalog::builtin().engines[0].id.clone();
        fake_installed(&dirs, &id);
        let err = install(&dirs, &id, false, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("already installed"), "{err}");
        let err = install(&dirs, "no-such-engine", false, &mut |_| {}).unwrap_err();
        assert!(err.to_string().contains("unknown engine"), "{err}");
    }

    #[test]
    fn gptk_import_from_a_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        let source = tmp.path().join("Game_Porting_Toolkit_3.0");
        std::fs::create_dir_all(source.join("lib/external")).unwrap();
        std::fs::write(source.join("lib/external/libd3dshared.dylib"), "x").unwrap();
        let info = import_gptk(&dirs, &source, None).unwrap();
        assert_eq!(info.version.as_deref(), Some("3.0"));
        assert_eq!(list(&dirs).unwrap().gptk, Some(info));
    }
}
