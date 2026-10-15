//! The engine catalog: a JSON manifest listing downloadable Wine builds and
//! graphics components, each with a URL and SHA-256.
//!
//! A catalog is embedded in the binary; `<data dir>/catalog.json` replaces it
//! when present, so entries can be updated without a new release.

use std::collections::HashSet;
use std::fs;

use cross_core::model::{GraphicsBackend, Translation};
use cross_core::paths::AppDirs;
use serde::Deserialize;

use crate::{Error, Result};

const BUILTIN: &str = include_str!("../catalog.json");
const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Archive {
    pub url: String,
    /// Lowercase hex SHA-256 of the archive.
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
}

impl Archive {
    /// Last URL path segment, used to name the cached download.
    pub fn file_name(&self) -> &str {
        let path = self.url.split(['?', '#']).next().unwrap_or(&self.url);
        path.rsplit('/').next().unwrap_or("archive")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct EngineEntry {
    pub id: String,
    pub name: String,
    pub wine_version: String,
    #[serde(default)]
    pub translation: Translation,
    pub archive: Archive,
    /// Graphics backends this engine can be used with.
    #[serde(default)]
    pub backends: Vec<GraphicsBackend>,
    /// Component ids installed with the DXVK + MoltenVK backend.
    #[serde(default)]
    pub components: Vec<String>,
    /// The build can execute Apple's D3DMetal (a CrossOver-derived Wine).
    /// Upstream Wine cannot: the payload loads and then every process using it
    /// dies (https://github.com/frankea/Whisky/issues/163). Absent = false.
    #[serde(default)]
    pub gptk_capable: bool,
}

impl EngineEntry {
    pub fn requires_rosetta(&self) -> bool {
        self.translation == Translation::Rosetta
    }

    pub fn supports_dxvk(&self) -> bool {
        self.backends.contains(&GraphicsBackend::Dxvk)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComponentKind {
    Dxvk,
    Moltenvk,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Component {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: ComponentKind,
    pub archive: Archive,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Catalog {
    pub schema: u32,
    pub engines: Vec<EngineEntry>,
    #[serde(default)]
    pub components: Vec<Component>,
}

impl Catalog {
    /// The catalog compiled into this binary.
    pub fn builtin() -> Self {
        Self::parse(BUILTIN).expect("embedded catalog.json is valid (checked by a unit test)")
    }

    /// `<data dir>/catalog.json` if it exists, otherwise the built-in catalog.
    pub fn load(dirs: &AppDirs) -> Result<Self> {
        let path = dirs.root().join("catalog.json");
        match fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text)
                .map_err(|e| Error::InvalidCatalog(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::builtin()),
            Err(e) => Err(Error::io(path, e)),
        }
    }

    pub fn parse(json: &str) -> Result<Self> {
        let catalog: Catalog =
            serde_json::from_str(json).map_err(|e| Error::InvalidCatalog(e.to_string()))?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn engine(&self, id: &str) -> Option<&EngineEntry> {
        self.engines.iter().find(|e| e.id == id)
    }

    pub fn component(&self, id: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }

    fn validate(&self) -> Result<()> {
        let bad = |msg: String| Err(Error::InvalidCatalog(msg));
        if self.schema != SCHEMA_VERSION {
            return bad(format!(
                "unsupported schema {} (expected {SCHEMA_VERSION})",
                self.schema
            ));
        }
        let mut seen = HashSet::new();
        let ids = self
            .engines
            .iter()
            .map(|e| &e.id)
            .chain(self.components.iter().map(|c| &c.id));
        for id in ids {
            validate_id(id)?;
            if !seen.insert(id.as_str()) {
                return bad(format!("duplicate id {id:?}"));
            }
        }
        let archives = self
            .engines
            .iter()
            .map(|e| (&e.id, &e.archive))
            .chain(self.components.iter().map(|c| (&c.id, &c.archive)));
        for (id, archive) in archives {
            if !archive.url.starts_with("https://") {
                return bad(format!("{id}: archive URL must use https"));
            }
            let hash = &archive.sha256;
            if hash.len() != 64 || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return bad(format!("{id}: sha256 must be 64 lowercase hex digits"));
            }
        }
        for engine in &self.engines {
            let mut kinds = Vec::new();
            for cid in &engine.components {
                let Some(component) = self.component(cid) else {
                    return bad(format!("{}: unknown component {cid:?}", engine.id));
                };
                kinds.push(component.kind);
            }
            if engine.supports_dxvk()
                && !(kinds.contains(&ComponentKind::Dxvk)
                    && kinds.contains(&ComponentKind::Moltenvk))
            {
                return bad(format!(
                    "{}: the dxvk backend needs both a dxvk and a moltenvk component",
                    engine.id
                ));
            }
        }
        Ok(())
    }
}

/// Engine and component ids become directory names: keep them path-safe.
pub fn validate_id(id: &str) -> Result<()> {
    let ok_char =
        |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.');
    let starts_ok = id
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if id.len() <= 64 && starts_ok && !id.contains("..") && id.chars().all(ok_char) {
        Ok(())
    } else {
        Err(Error::InvalidId(id.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_is_valid_and_complete() {
        let catalog = Catalog::builtin();
        assert!(!catalog.engines.is_empty());
        let engine = catalog.engine("wine-devel-11.18").unwrap();
        assert!(engine.requires_rosetta());
        assert!(engine.supports_dxvk());
        assert_eq!(engine.archive.file_name(), "wine-devel-11.18-osx64.tar.xz");
        for cid in &engine.components {
            assert!(catalog.component(cid).is_some());
        }
    }

    #[test]
    fn rejects_bad_ids() {
        for bad in ["", "../x", "a/b", "A", ".hidden", "a..b", &"x".repeat(65)] {
            assert!(validate_id(bad).is_err(), "{bad:?}");
        }
        for ok in ["wine-devel-11.18", "dxvk_1", "0a"] {
            assert!(validate_id(ok).is_ok(), "{ok}");
        }
    }

    fn catalog_with(mutate: impl FnOnce(&mut serde_json::Value)) -> Result<Catalog> {
        let mut value: serde_json::Value = serde_json::from_str(BUILTIN).unwrap();
        mutate(&mut value);
        Catalog::parse(&value.to_string())
    }

    #[test]
    fn rejects_non_https_urls() {
        let err = catalog_with(|v| {
            v["engines"][0]["archive"]["url"] = "http://example.com/a.tar.xz".into();
        })
        .unwrap_err();
        assert!(err.to_string().contains("https"), "{err}");
    }

    #[test]
    fn rejects_malformed_checksums() {
        for bad in ["abc", &"A".repeat(64), &"g".repeat(64)] {
            let err = catalog_with(|v| {
                v["engines"][0]["archive"]["sha256"] = bad.to_string().into();
            })
            .unwrap_err();
            assert!(err.to_string().contains("sha256"), "{bad}: {err}");
        }
    }

    #[test]
    fn rejects_duplicates_and_dangling_components() {
        let err = catalog_with(|v| {
            v["engines"][1]["id"] = "wine-devel-11.18".into();
        })
        .unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");

        let err = catalog_with(|v| {
            v["engines"][0]["components"] = serde_json::json!(["nope"]);
        })
        .unwrap_err();
        assert!(err.to_string().contains("unknown component"), "{err}");
    }

    #[test]
    fn dxvk_backend_needs_both_components() {
        let err = catalog_with(|v| {
            v["engines"][0]["components"] = serde_json::json!(["dxvk-macos-1.10.3"]);
        })
        .unwrap_err();
        assert!(err.to_string().contains("moltenvk"), "{err}");
    }

    #[test]
    fn rejects_unknown_schema() {
        let err = catalog_with(|v| v["schema"] = 2.into()).unwrap_err();
        assert!(err.to_string().contains("schema"), "{err}");
    }

    #[test]
    fn local_catalog_overrides_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        assert_eq!(Catalog::load(&dirs).unwrap(), Catalog::builtin());

        fs::write(
            dirs.root().join("catalog.json"),
            r#"{"schema":1,"engines":[]}"#,
        )
        .unwrap();
        assert!(Catalog::load(&dirs).unwrap().engines.is_empty());

        fs::write(dirs.root().join("catalog.json"), "{").unwrap();
        assert!(Catalog::load(&dirs).is_err());
    }
}
