//! Installing, listing and removing engines under `<data dir>/engines/<id>/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cross_core::model::{Engine, GraphicsBackend, Translation};
use cross_core::paths::AppDirs;
use serde::{Deserialize, Serialize};

use crate::catalog::{validate_id, Catalog, EngineEntry};
use crate::download::{extract_archive, fetch_verified};
use crate::{Error, Progress, Result};

/// File inside each engine directory describing what was installed.
pub const ENGINE_FILE: &str = "engine.toml";
/// Subdirectory holding graphics components (DXVK, MoltenVK).
pub const COMPONENTS_DIR: &str = "components";

/// What `engine.toml` records about an installed engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledEngine {
    pub id: String,
    pub name: String,
    pub wine_version: String,
    #[serde(default)]
    pub translation: Translation,
    /// Path of the `wine`/`wine64` executable relative to the engine directory.
    pub wine_bin: PathBuf,
    #[serde(default)]
    pub backends: Vec<GraphicsBackend>,
    /// Ids of components installed under `components/`.
    #[serde(default)]
    pub components: Vec<String>,
    /// SHA-256 of the archive this engine was installed from.
    pub source_sha256: String,
    pub installed_unix: u64,
}

impl InstalledEngine {
    pub fn engine(&self) -> Engine {
        Engine {
            id: self.id.clone(),
            wine_version: self.wine_version.clone(),
            translation: self.translation,
        }
    }

    pub fn has_dxvk(&self) -> bool {
        !self.components.is_empty()
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct InstallOptions {
    /// Also install the DXVK + MoltenVK components.
    pub with_dxvk: bool,
}

pub struct EngineManager<'a> {
    dirs: &'a AppDirs,
}

impl<'a> EngineManager<'a> {
    pub fn new(dirs: &'a AppDirs) -> Self {
        Self { dirs }
    }

    /// Absolute path of an installed engine's wine executable.
    pub fn wine_path(&self, engine: &InstalledEngine) -> PathBuf {
        self.dirs.engine_dir(&engine.id).join(&engine.wine_bin)
    }

    pub fn get(&self, id: &str) -> Result<Option<InstalledEngine>> {
        validate_id(id)?;
        let file = self.dirs.engine_dir(id).join(ENGINE_FILE);
        match fs::read_to_string(&file) {
            Ok(text) => toml::from_str(&text)
                .map(Some)
                .map_err(|source| Error::TomlParse { path: file, source }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(file, e)),
        }
    }

    /// Installed engines sorted by id. Half-installed staging directories are ignored.
    pub fn list(&self) -> Result<Vec<InstalledEngine>> {
        let root = self.dirs.engines_dir();
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io(&root, e)),
        };
        let mut engines = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(&root, e))?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if validate_id(&id).is_err() {
                continue;
            }
            if let Some(engine) = self.get(&id)? {
                engines.push(engine);
            }
        }
        engines.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(engines)
    }

    /// Download (or reuse the cached download of), verify and unpack an engine.
    ///
    /// Everything is unpacked into a staging directory that is renamed into
    /// place at the end, so a failure never leaves a half-installed engine.
    pub fn install(
        &self,
        catalog: &Catalog,
        id: &str,
        options: InstallOptions,
        progress: &mut dyn FnMut(Progress<'_>),
    ) -> Result<InstalledEngine> {
        let entry = catalog
            .engine(id)
            .ok_or_else(|| Error::UnknownEngine(id.to_owned()))?;
        if options.with_dxvk && !entry.supports_dxvk() {
            return Err(Error::InvalidCatalog(format!(
                "{id} does not support the dxvk backend"
            )));
        }
        if self.get(id)?.is_some() {
            return Err(Error::AlreadyInstalled(id.to_owned()));
        }

        let engines_dir = self.dirs.engines_dir();
        fs::create_dir_all(&engines_dir).map_err(|e| Error::io(&engines_dir, e))?;
        let staging = engines_dir.join(format!(".{id}.installing"));
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|e| Error::io(&staging, e))?;
        }

        let result = self.install_into(catalog, entry, options, &staging, progress);
        match result {
            Ok(installed) => {
                let target = self.dirs.engine_dir(id);
                if let Err(e) = fs::rename(&staging, &target) {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(Error::io(target, e));
                }
                tracing::info!(%id, "engine installed");
                Ok(installed)
            }
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                Err(e)
            }
        }
    }

    fn install_into(
        &self,
        catalog: &Catalog,
        entry: &EngineEntry,
        options: InstallOptions,
        staging: &Path,
        progress: &mut dyn FnMut(Progress<'_>),
    ) -> Result<InstalledEngine> {
        let cache = self.dirs.downloads_dir();

        let archive = fetch_verified(&entry.archive, &entry.id, &cache, progress)?;
        progress(Progress::Extracting { label: &entry.id });
        extract_archive(&archive, staging)?;
        let wine_bin = find_wine_binary(staging)?;

        let mut components = Vec::new();
        if options.with_dxvk {
            for cid in &entry.components {
                let component = catalog.component(cid).ok_or_else(|| {
                    Error::InvalidCatalog(format!("{}: unknown component {cid:?}", entry.id))
                })?;
                let archive = fetch_verified(&component.archive, cid, &cache, progress)?;
                progress(Progress::Extracting { label: cid });
                extract_archive(&archive, &staging.join(COMPONENTS_DIR).join(cid))?;
                components.push(cid.clone());
            }
        }

        let installed = InstalledEngine {
            id: entry.id.clone(),
            name: entry.name.clone(),
            wine_version: entry.wine_version.clone(),
            translation: entry.translation,
            wine_bin,
            backends: entry.backends.clone(),
            components,
            source_sha256: entry.archive.sha256.clone(),
            installed_unix: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        };
        let file = staging.join(ENGINE_FILE);
        fs::write(&file, toml::to_string_pretty(&installed)?).map_err(|e| Error::io(file, e))?;
        Ok(installed)
    }

    /// Add the DXVK + MoltenVK components to an engine that is already installed
    /// (it was installed without `--dxvk`). Components are unpacked next to the
    /// engine and recorded in its `engine.toml`; a failure changes nothing.
    pub fn add_dxvk(
        &self,
        catalog: &Catalog,
        id: &str,
        progress: &mut dyn FnMut(Progress<'_>),
    ) -> Result<InstalledEngine> {
        let mut installed = self
            .get(id)?
            .ok_or_else(|| Error::NotInstalled(id.to_owned()))?;
        let entry = catalog
            .engine(id)
            .ok_or_else(|| Error::UnknownEngine(id.to_owned()))?;
        if !entry.supports_dxvk() {
            return Err(Error::InvalidCatalog(format!(
                "{id} does not support the dxvk backend"
            )));
        }

        let dir = self.dirs.engine_dir(id);
        let staging = dir.join(format!(".{COMPONENTS_DIR}.installing"));
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(|e| Error::io(&staging, e))?;
        }
        let result = self.unpack_components(catalog, entry, &staging, progress);
        let ids = match result {
            Ok(ids) => ids,
            Err(e) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(e);
            }
        };

        let target = dir.join(COMPONENTS_DIR);
        for cid in &ids {
            let from = staging.join(cid);
            let to = target.join(cid);
            let moved = fs::create_dir_all(&target)
                .and_then(|()| {
                    if to.exists() {
                        fs::remove_dir_all(&to)?;
                    }
                    fs::rename(&from, &to)
                })
                .map_err(|e| Error::io(&to, e));
            if let Err(e) = moved {
                let _ = fs::remove_dir_all(&staging);
                return Err(e);
            }
        }
        let _ = fs::remove_dir_all(&staging);

        installed.components = ids;
        let file = dir.join(ENGINE_FILE);
        fs::write(&file, toml::to_string_pretty(&installed)?).map_err(|e| Error::io(file, e))?;
        Ok(installed)
    }

    fn unpack_components(
        &self,
        catalog: &Catalog,
        entry: &EngineEntry,
        staging: &Path,
        progress: &mut dyn FnMut(Progress<'_>),
    ) -> Result<Vec<String>> {
        let cache = self.dirs.downloads_dir();
        let mut ids = Vec::new();
        for cid in &entry.components {
            let component = catalog.component(cid).ok_or_else(|| {
                Error::InvalidCatalog(format!("{}: unknown component {cid:?}", entry.id))
            })?;
            let archive = fetch_verified(&component.archive, cid, &cache, progress)?;
            progress(Progress::Extracting { label: cid });
            extract_archive(&archive, &staging.join(cid))?;
            ids.push(cid.clone());
        }
        Ok(ids)
    }

    /// Delete an installed engine. Bottles that reference it are not touched.
    pub fn remove(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        if self.get(id)?.is_none() {
            return Err(Error::NotInstalled(id.to_owned()));
        }
        let dir = self.dirs.engine_dir(id);
        fs::remove_dir_all(&dir).map_err(|e| Error::io(dir, e))?;
        tracing::info!(%id, "engine removed");
        Ok(())
    }
}

/// Locate `bin/wine64` (preferred) or `bin/wine` inside an unpacked engine,
/// returning the shallowest match relative to `root`. Symlinked directories are
/// not followed.
pub fn find_wine_binary(root: &Path) -> Result<PathBuf> {
    const MAX_DEPTH: usize = 8;
    let mut level = vec![root.to_owned()];
    for _ in 0..MAX_DEPTH {
        let mut best: Option<PathBuf> = None;
        let mut next = Vec::new();
        for dir in &level {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    next.push(path);
                } else if dir.file_name().is_some_and(|n| n == "bin") {
                    let name = entry.file_name();
                    let is_wine64 = name == "wine64";
                    if (is_wine64 || name == "wine") && (best.is_none() || is_wine64) {
                        best = Some(path);
                    }
                }
            }
        }
        if let Some(path) = best {
            return Ok(path.strip_prefix(root).unwrap_or(&path).to_owned());
        }
        level = next;
        if level.is_empty() {
            break;
        }
    }
    Err(Error::WineNotFound(root.to_owned()))
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;
    use crate::catalog::{Archive, Component, ComponentKind};
    use crate::download::test_support::{serve, sha256_hex};

    /// Build a tar.gz holding `files` (relative path, contents).
    fn make_tar_gz(dir: &Path, name: &str, files: &[(&str, &str)]) -> Vec<u8> {
        let src = dir.join(format!("{name}-src"));
        for (path, contents) in files {
            let full = src.join(path);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, contents).unwrap();
        }
        let archive = dir.join(format!("{name}.tar.gz"));
        assert!(Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&src)
            .arg(".")
            .status()
            .unwrap()
            .success());
        fs::read(archive).unwrap()
    }

    fn archive_at(base: &str, name: &str, body: &[u8]) -> Archive {
        Archive {
            url: format!("{base}/{name}.tar.gz"),
            sha256: sha256_hex(body),
            size: None,
        }
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        dirs: AppDirs,
        catalog: Catalog,
    }

    /// A catalog with one engine and DXVK/MoltenVK components served locally.
    /// `connections` is how many downloads each archive may serve.
    fn fixture(connections: usize) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let build = tmp.path().join("build");
        let engine_tar = make_tar_gz(
            &build,
            "engine",
            &[("Wine.app/Contents/Resources/wine/bin/wine64", "#!/bin/sh\n")],
        );
        let dxvk_tar = make_tar_gz(&build, "dxvk", &[("x64/d3d11.dll", "dll")]);
        let mvk_tar = make_tar_gz(&build, "mvk", &[("libMoltenVK.dylib", "lib")]);

        let engine_base = serve(engine_tar.clone(), connections);
        let dxvk_base = serve(dxvk_tar.clone(), connections);
        let mvk_base = serve(mvk_tar.clone(), connections);

        let component = |id: &str, kind, base: &str, tar: &[u8]| Component {
            id: id.to_owned(),
            name: id.to_owned(),
            version: "1".into(),
            kind,
            archive: archive_at(base, id, tar),
        };
        let catalog = Catalog {
            schema: 1,
            engines: vec![EngineEntry {
                id: "wine-test-1".into(),
                name: "Wine Test".into(),
                wine_version: "1.0".into(),
                translation: Translation::Rosetta,
                archive: archive_at(&engine_base, "engine", &engine_tar),
                backends: vec![GraphicsBackend::D3dMetal, GraphicsBackend::Dxvk],
                components: vec!["dxvk".into(), "mvk".into()],
            }],
            components: vec![
                component("dxvk", ComponentKind::Dxvk, &dxvk_base, &dxvk_tar),
                component("mvk", ComponentKind::Moltenvk, &mvk_base, &mvk_tar),
            ],
        };
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        Fixture {
            _tmp: tmp,
            dirs,
            catalog,
        }
    }

    #[test]
    fn install_list_remove_round_trip() {
        let fx = fixture(1);
        let manager = EngineManager::new(&fx.dirs);
        assert!(manager.list().unwrap().is_empty());

        let mut steps = Vec::new();
        let installed = manager
            .install(
                &fx.catalog,
                "wine-test-1",
                InstallOptions::default(),
                &mut |p| steps.push(format!("{p:?}")),
            )
            .unwrap();
        assert!(steps.iter().any(|s| s.contains("Extracting")));
        assert_eq!(
            installed.wine_bin,
            Path::new("Wine.app/Contents/Resources/wine/bin/wine64")
        );
        assert!(installed.components.is_empty());
        assert!(manager.wine_path(&installed).is_file());
        assert_eq!(installed.engine().translation, Translation::Rosetta);

        assert_eq!(manager.list().unwrap(), vec![installed.clone()]);
        assert_eq!(manager.get("wine-test-1").unwrap(), Some(installed));

        let err = manager
            .install(
                &fx.catalog,
                "wine-test-1",
                InstallOptions::default(),
                &mut |_| {},
            )
            .unwrap_err();
        assert!(matches!(err, Error::AlreadyInstalled(_)), "{err}");

        manager.remove("wine-test-1").unwrap();
        assert!(manager.list().unwrap().is_empty());
        assert!(!fx.dirs.engine_dir("wine-test-1").exists());
        assert!(matches!(
            manager.remove("wine-test-1").unwrap_err(),
            Error::NotInstalled(_)
        ));
    }

    #[test]
    fn reinstall_after_remove_uses_the_download_cache() {
        // Each archive is served exactly once; the second install must work offline.
        let fx = fixture(1);
        let manager = EngineManager::new(&fx.dirs);
        let opts = InstallOptions { with_dxvk: true };
        manager
            .install(&fx.catalog, "wine-test-1", opts, &mut |_| {})
            .unwrap();
        manager.remove("wine-test-1").unwrap();

        let mut cache_hits = 0;
        manager
            .install(&fx.catalog, "wine-test-1", opts, &mut |p| {
                cache_hits += usize::from(matches!(p, Progress::CacheHit { .. }));
            })
            .unwrap();
        assert_eq!(cache_hits, 3);
    }

    #[test]
    fn dxvk_components_land_under_the_engine() {
        let fx = fixture(1);
        let manager = EngineManager::new(&fx.dirs);
        let installed = manager
            .install(
                &fx.catalog,
                "wine-test-1",
                InstallOptions { with_dxvk: true },
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(installed.components, ["dxvk", "mvk"]);
        assert!(installed.has_dxvk());
        let root = fx.dirs.engine_dir("wine-test-1").join(COMPONENTS_DIR);
        assert!(root.join("dxvk/x64/d3d11.dll").is_file());
        assert!(root.join("mvk/libMoltenVK.dylib").is_file());
    }

    #[test]
    fn dxvk_can_be_added_to_an_installed_engine() {
        let fx = fixture(1);
        let manager = EngineManager::new(&fx.dirs);
        manager
            .install(
                &fx.catalog,
                "wine-test-1",
                InstallOptions::default(),
                &mut |_| {},
            )
            .unwrap();
        assert!(!manager.get("wine-test-1").unwrap().unwrap().has_dxvk());

        let updated = manager
            .add_dxvk(&fx.catalog, "wine-test-1", &mut |_| {})
            .unwrap();
        assert_eq!(updated.components, ["dxvk", "mvk"]);
        assert_eq!(manager.get("wine-test-1").unwrap().unwrap(), updated);
        let root = fx.dirs.engine_dir("wine-test-1").join(COMPONENTS_DIR);
        assert!(root.join("dxvk/x64/d3d11.dll").is_file());
        assert!(root.join("mvk/libMoltenVK.dylib").is_file());
        assert!(!fx
            .dirs
            .engine_dir("wine-test-1")
            .join(".components.installing")
            .exists());

        assert!(matches!(
            manager
                .add_dxvk(&fx.catalog, "nope", &mut |_| {})
                .unwrap_err(),
            Error::NotInstalled(_)
        ));
    }

    #[test]
    fn failed_add_dxvk_changes_nothing() {
        let mut fx = fixture(1);
        let manager = EngineManager::new(&fx.dirs);
        manager
            .install(
                &fx.catalog,
                "wine-test-1",
                InstallOptions::default(),
                &mut |_| {},
            )
            .unwrap();
        fx.catalog.components[1].archive.sha256 = "0".repeat(64);
        let err = manager
            .add_dxvk(&fx.catalog, "wine-test-1", &mut |_| {})
            .unwrap_err();
        assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err}");
        assert!(!manager.get("wine-test-1").unwrap().unwrap().has_dxvk());
        assert!(!fx
            .dirs
            .engine_dir("wine-test-1")
            .join(COMPONENTS_DIR)
            .exists());
    }

    #[test]
    fn failed_install_leaves_no_trace() {
        let mut fx = fixture(1);
        fx.catalog.engines[0].archive.sha256 = "0".repeat(64);
        let manager = EngineManager::new(&fx.dirs);
        let err = manager
            .install(
                &fx.catalog,
                "wine-test-1",
                InstallOptions::default(),
                &mut |_| {},
            )
            .unwrap_err();
        assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err}");
        assert!(manager.list().unwrap().is_empty());
        let leftovers: Vec<_> = fs::read_dir(fx.dirs.engines_dir())
            .map(|d| d.flatten().map(|e| e.file_name()).collect())
            .unwrap_or_default();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn archive_without_wine_is_rejected_and_cleaned_up() {
        let fx = fixture(1);
        let tmp = tempfile::tempdir().unwrap();
        let body = make_tar_gz(tmp.path(), "empty", &[("readme.txt", "no wine here")]);
        let base = serve(body.clone(), 1);
        let mut catalog = fx.catalog.clone();
        catalog.engines[0].archive = archive_at(&base, "empty", &body);

        let manager = EngineManager::new(&fx.dirs);
        let err = manager
            .install(
                &catalog,
                "wine-test-1",
                InstallOptions::default(),
                &mut |_| {},
            )
            .unwrap_err();
        assert!(matches!(err, Error::WineNotFound(_)), "{err}");
        assert!(!fx.dirs.engine_dir("wine-test-1").exists());
        assert!(!fx
            .dirs
            .engines_dir()
            .join(".wine-test-1.installing")
            .exists());
    }

    #[test]
    fn unknown_engine_and_unsupported_backend() {
        let mut fx = fixture(0);
        let manager = EngineManager::new(&fx.dirs);
        assert!(matches!(
            manager
                .install(&fx.catalog, "nope", InstallOptions::default(), &mut |_| {})
                .unwrap_err(),
            Error::UnknownEngine(_)
        ));

        fx.catalog.engines[0].backends = vec![GraphicsBackend::D3dMetal];
        assert!(manager
            .install(
                &fx.catalog,
                "wine-test-1",
                InstallOptions { with_dxvk: true },
                &mut |_| {},
            )
            .is_err());
    }

    #[test]
    fn wine_binary_prefers_wine64_and_shallowest_match() {
        let tmp = tempfile::tempdir().unwrap();
        for p in ["a/bin/wine", "a/bin/wine64", "a/b/c/bin/wine64"] {
            let full = tmp.path().join(p);
            fs::create_dir_all(full.parent().unwrap()).unwrap();
            fs::write(full, "").unwrap();
        }
        assert_eq!(
            find_wine_binary(tmp.path()).unwrap(),
            Path::new("a/bin/wine64")
        );
    }
}
