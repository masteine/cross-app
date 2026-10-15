//! Persistent configuration: global `config.toml` and per-bottle `bottle.toml`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{validate_bottle_id, Bottle};
use crate::paths::AppDirs;
use crate::{Error, Result};

/// Global settings stored in `config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Engine used for new bottles when none is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_engine: Option<String>,
    /// Where imported GPTK libraries live, when not the default location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gptk_dir: Option<PathBuf>,
    /// Detail kept in log files (`debug` by default; the terminal is controlled by
    /// `-v`/`-q`). Overridden by `RUST_LOG`.
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// Where `cross steam patches --update` fetches `steam-patches.toml` from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_patches_url: Option<String>,
}

fn default_log_level() -> String {
    "debug".to_owned()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_engine: None,
            gptk_dir: None,
            log_level: default_log_level(),
            steam_patches_url: None,
        }
    }
}

impl Config {
    /// Load `config.toml`, returning defaults when the file does not exist yet.
    pub fn load(dirs: &AppDirs) -> Result<Self> {
        match read_toml(&dirs.config_file()) {
            Ok(config) => Ok(config),
            Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::default())
            }
            Err(err) => Err(err),
        }
    }

    pub fn save(&self, dirs: &AppDirs) -> Result<()> {
        write_toml(&dirs.config_file(), self)
    }

    /// Effective GPTK directory: explicit setting or the default location.
    pub fn gptk_dir(&self, dirs: &AppDirs) -> PathBuf {
        self.gptk_dir.clone().unwrap_or_else(|| dirs.gptk_dir())
    }
}

/// Reads and writes `bottle.toml` files under `bottles/<id>/`.
pub struct BottleStore<'a> {
    dirs: &'a AppDirs,
}

impl<'a> BottleStore<'a> {
    pub fn new(dirs: &'a AppDirs) -> Self {
        Self { dirs }
    }

    pub fn load(&self, id: &str) -> Result<Bottle> {
        validate_bottle_id(id)?;
        read_toml(&self.dirs.bottle_file(id))
    }

    pub fn save(&self, bottle: &Bottle) -> Result<()> {
        validate_bottle_id(&bottle.id)?;
        write_toml(&self.dirs.bottle_file(&bottle.id), bottle)
    }

    /// All bottles, sorted by id. Directories without a `bottle.toml` are skipped,
    /// and so (with a warning) is a `bottle.toml` that cannot be read, so one
    /// broken bottle does not hide all the others.
    pub fn list(&self) -> Result<Vec<Bottle>> {
        let root = self.dirs.bottles_dir();
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(Error::io(&root, err)),
        };
        let mut bottles = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(&root, e))?;
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if validate_bottle_id(&id).is_err() || !self.dirs.bottle_file(&id).is_file() {
                continue;
            }
            match self.load(&id) {
                Ok(bottle) => bottles.push(bottle),
                Err(err) => {
                    tracing::warn!(bottle = %id, error = %err, "skipping unreadable bottle")
                }
            }
        }
        bottles.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(bottles)
    }
}

fn read_toml<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    toml::from_str(&text).map_err(|source| Error::TomlParse {
        path: path.to_owned(),
        source,
    })
}

/// Write via a temp file + rename so a crash never leaves a half-written config.
fn write_toml<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text = toml::to_string_pretty(value)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).map_err(|e| Error::io(&tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{App, DepRecord, DepSource, GraphicsBackend, LaunchProfile};

    #[test]
    fn missing_config_yields_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        assert_eq!(Config::load(&dirs).unwrap(), Config::default());
    }

    #[test]
    fn config_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let config = Config {
            default_engine: Some("wine-11".into()),
            gptk_dir: Some("/opt/gptk".into()),
            log_level: "debug".into(),
            steam_patches_url: Some("https://example.com/steam-patches.toml".into()),
        };
        config.save(&dirs).unwrap();
        assert_eq!(Config::load(&dirs).unwrap(), config);
    }

    #[test]
    fn malformed_config_reports_path() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        fs::write(dirs.config_file(), "log_level = [").unwrap();
        let err = Config::load(&dirs).unwrap_err();
        assert!(matches!(err, Error::TomlParse { .. }), "{err}");
        assert!(err.to_string().contains("config.toml"));
    }

    #[test]
    fn bottle_round_trips_with_apps() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let store = BottleStore::new(&dirs);

        let mut bottle = Bottle::new("steam", "Steam").unwrap();
        bottle.engine = Some("wine-11".into());
        bottle.backend = GraphicsBackend::Dxvk;
        let mut profile = LaunchProfile::default();
        profile.env.insert("WINEMSYNC".into(), "1".into());
        bottle.apps.push(App {
            name: "Hollow Knight".into(),
            path: "C:\\Program Files\\Steam\\steam.exe".into(),
            steam_appid: Some(367520),
            profile,
        });

        bottle.deps.insert(
            "vcrun2022".into(),
            DepRecord {
                installed_unix: 1,
                source: DepSource::Catalog,
            },
        );

        store.save(&bottle).unwrap();
        assert_eq!(store.load("steam").unwrap(), bottle);
        let text = fs::read_to_string(dirs.bottle_file("steam")).unwrap();
        assert!(text.contains("[deps.vcrun2022]"), "{text}");
    }

    #[test]
    fn list_is_sorted_and_ignores_strays() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let store = BottleStore::new(&dirs);
        assert!(store.list().unwrap().is_empty());

        store.save(&Bottle::new("zeta", "Zeta").unwrap()).unwrap();
        store.save(&Bottle::new("alpha", "Alpha").unwrap()).unwrap();
        fs::create_dir_all(dirs.bottle_dir("no-toml")).unwrap();
        fs::write(dirs.bottles_dir().join("stray-file"), "").unwrap();

        let ids: Vec<_> = store.list().unwrap().into_iter().map(|b| b.id).collect();
        assert_eq!(ids, ["alpha", "zeta"]);

        // A corrupt bottle.toml is skipped, not fatal for the whole list.
        fs::create_dir_all(dirs.bottle_dir("broken")).unwrap();
        fs::write(dirs.bottle_file("broken"), "id = [").unwrap();
        let ids: Vec<_> = store.list().unwrap().into_iter().map(|b| b.id).collect();
        assert_eq!(ids, ["alpha", "zeta"]);
        assert!(store.load("broken").is_err());
    }

    #[test]
    fn load_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let err = BottleStore::new(&dirs).load("../etc").unwrap_err();
        assert!(matches!(err, Error::InvalidBottleId(_)));
    }
}
