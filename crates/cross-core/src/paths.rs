use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::{Error, Result};

/// Environment variable that relocates the whole data directory (used by tests and CI).
pub const HOME_OVERRIDE_ENV: &str = "CROSS_APP_HOME";

/// All on-disk locations used by cross-app.
///
/// On macOS the default root is `~/Library/Application Support/cross-app`.
#[derive(Debug, Clone)]
pub struct AppDirs {
    root: PathBuf,
}

impl AppDirs {
    /// Resolve the root from `CROSS_APP_HOME` or the platform default.
    pub fn discover() -> Result<Self> {
        if let Some(custom) = std::env::var_os(HOME_OVERRIDE_ENV).filter(|v| !v.is_empty()) {
            return Ok(Self::with_root(custom));
        }
        let dirs = ProjectDirs::from("", "", "cross-app").ok_or(Error::NoHomeDir)?;
        Ok(Self::with_root(dirs.data_dir()))
    }

    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.toml")
    }

    /// Downloaded Wine engines: `engines/<id>/`.
    pub fn engines_dir(&self) -> PathBuf {
        self.root.join("engines")
    }

    pub fn engine_dir(&self, id: &str) -> PathBuf {
        self.engines_dir().join(id)
    }

    /// Verified downloads, keyed by SHA-256, reused for offline reinstalls.
    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("cache").join("downloads")
    }

    /// Imported Game Porting Toolkit libraries.
    pub fn gptk_dir(&self) -> PathBuf {
        self.root.join("gptk")
    }

    pub fn bottles_dir(&self) -> PathBuf {
        self.root.join("bottles")
    }

    pub fn bottle_dir(&self, id: &str) -> PathBuf {
        self.bottles_dir().join(id)
    }

    pub fn bottle_file(&self, id: &str) -> PathBuf {
        self.bottle_dir(id).join("bottle.toml")
    }

    /// The Wine prefix (`WINEPREFIX`) of a bottle.
    pub fn bottle_prefix(&self, id: &str) -> PathBuf {
        self.bottle_dir(id).join("prefix")
    }

    pub fn bottle_logs_dir(&self, id: &str) -> PathBuf {
        self.bottle_dir(id).join("logs")
    }

    /// Logs not tied to a bottle (engine downloads, `cross doctor`, ...).
    pub fn global_logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottle_paths_nest_under_root() {
        let dirs = AppDirs::with_root("/data");
        assert_eq!(dirs.config_file(), Path::new("/data/config.toml"));
        assert_eq!(
            dirs.bottle_file("demo"),
            Path::new("/data/bottles/demo/bottle.toml")
        );
        assert_eq!(
            dirs.bottle_prefix("demo"),
            Path::new("/data/bottles/demo/prefix")
        );
        assert_eq!(
            dirs.bottle_logs_dir("demo"),
            Path::new("/data/bottles/demo/logs")
        );
    }
}
