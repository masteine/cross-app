//! Domain types: engines, bottles, apps and launch profiles.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// How x86_64 Windows code ends up running on Apple Silicon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Translation {
    /// Intel Wine build executed through Rosetta 2.
    #[default]
    Rosetta,
    /// ARM64EC Wine with a bundled x86 emulator (FEX and friends).
    Native,
    /// An engine owned by another product (e.g. CrossOver) that we only drive.
    External,
}

impl Translation {
    /// The engine's Wine is x86_64 code that only runs through Rosetta 2.
    pub fn requires_rosetta(self) -> bool {
        self == Translation::Rosetta
    }

    /// Engines of this kind are downloaded from the catalog; external engines
    /// are found on the system instead.
    pub fn is_downloaded(self) -> bool {
        self != Translation::External
    }

    /// Name used in manifests, `engine.toml` and CLI output.
    pub fn name(self) -> &'static str {
        match self {
            Translation::Rosetta => "rosetta",
            Translation::Native => "native",
            Translation::External => "external",
        }
    }
}

/// How Direct3D calls reach Metal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum GraphicsBackend {
    /// Apple's D3DMetal from the Game Porting Toolkit (DX11/DX12).
    #[default]
    D3dMetal,
    /// DXVK on top of MoltenVK (DX9/10/11).
    Dxvk,
}

/// Which Wine synchronization primitive to ask for. Only engines built with the
/// esync/msync patches read the variables (see `cross_engine::features`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
    /// Neither variable is set.
    None,
    /// `WINEESYNC=1`
    Esync,
    /// `WINEMSYNC=1` (mach-semaphore sync on macOS)
    Msync,
}

/// Graphics and performance switches, stored per bottle (`[graphics]` in
/// `bottle.toml`) and per program (`LaunchProfile::graphics`). `None` means
/// "inherit": a program's value wins over the bottle's, which wins over the
/// built-in default (off).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GraphicsSettings {
    /// Apple's Metal performance HUD (`MTL_HUD_ENABLED=1`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metal_hud: Option<bool>,
    /// Per-frame Metal HUD statistics written to the console once per second
    /// (`MTL_HUD_LOG_ENABLED=1`, requires the HUD); used for FPS logs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metal_hud_log: Option<bool>,
    /// DXVK HUD spec, e.g. `fps,frametimes` (`DXVK_HUD`); empty = off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dxvk_hud: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncMode>,
    /// Number of CPUs reported to Windows programs, for old games that break on
    /// many cores. Only engines that read `WINENCPU` honour it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_count: Option<u32>,
}

impl GraphicsSettings {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// `self` with every unset field taken from `base`.
    pub fn or(&self, base: &GraphicsSettings) -> GraphicsSettings {
        GraphicsSettings {
            metal_hud: self.metal_hud.or(base.metal_hud),
            metal_hud_log: self.metal_hud_log.or(base.metal_hud_log),
            dxvk_hud: self.dxvk_hud.clone().or_else(|| base.dxvk_hud.clone()),
            sync: self.sync.or(base.sync),
            cpu_count: self.cpu_count.or(base.cpu_count),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WindowsVersion {
    Win7,
    Win8,
    Win10,
    #[default]
    Win11,
}

impl WindowsVersion {
    /// Value Wine stores in `HKCU\\Software\\Wine\\Version`.
    pub fn wine_name(self) -> &'static str {
        match self {
            WindowsVersion::Win7 => "win7",
            WindowsVersion::Win8 => "win8",
            WindowsVersion::Win10 => "win10",
            WindowsVersion::Win11 => "win11",
        }
    }
}

/// An installed Wine build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Engine {
    pub id: String,
    pub wine_version: String,
    #[serde(default)]
    pub translation: Translation,
}

impl Engine {
    pub fn requires_rosetta(&self) -> bool {
        self.translation.requires_rosetta()
    }
}

/// Per-program settings applied when launching an executable inside a bottle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LaunchProfile {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<PathBuf>,
    /// Overrides the bottle's graphics backend for this program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<GraphicsBackend>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// DLL name -> mode (`n`, `b`, `n,b`, `d` ...).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dll_overrides: BTreeMap<String, String>,
    /// Overrides the bottle's graphics settings for this program.
    #[serde(default, skip_serializing_if = "GraphicsSettings::is_empty")]
    pub graphics: GraphicsSettings,
}

impl LaunchProfile {
    /// Value for `WINEDLLOVERRIDES`, e.g. `d3d11=n,b;dxgi=n`. `None` when empty.
    pub fn wine_dll_overrides(&self) -> Option<String> {
        if self.dll_overrides.is_empty() {
            return None;
        }
        Some(
            self.dll_overrides
                .iter()
                .map(|(dll, mode)| format!("{dll}={mode}"))
                .collect::<Vec<_>>()
                .join(";"),
        )
    }

    /// Environment to set for the child process: `env` plus `WINEDLLOVERRIDES`.
    /// An explicit `WINEDLLOVERRIDES` in `env` wins.
    pub fn launch_env(&self) -> BTreeMap<String, String> {
        let mut env = self.env.clone();
        if let Some(overrides) = self.wine_dll_overrides() {
            env.entry("WINEDLLOVERRIDES".to_owned())
                .or_insert(overrides);
        }
        env
    }
}

/// A launchable program registered in a bottle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct App {
    pub name: String,
    /// Path to the `.exe`/`.msi`/`.lnk`, usually a Windows path inside the prefix.
    pub path: PathBuf,
    /// Steam appid, when the app comes from Steam.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_appid: Option<u32>,
    #[serde(default)]
    pub profile: LaunchProfile,
}

/// Where a dependency came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DepSource {
    /// Our own verb catalog.
    #[default]
    Catalog,
    /// The system `winetricks` (fallback for verbs we do not carry).
    Winetricks,
}

/// A dependency installed into a bottle (`[deps.<verb>]` in `bottle.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepRecord {
    pub installed_unix: u64,
    #[serde(default)]
    pub source: DepSource,
}

/// Steam installed into a bottle by `cross steam install` (`[steam]` in `bottle.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SteamRecord {
    /// Revision of `steam-patches.toml` last applied to this bottle.
    pub patches_revision: u32,
    /// SHA-256 of the `SteamSetup.exe` that was run.
    pub installer_sha256: String,
    pub installed_unix: u64,
}

/// An isolated Wine prefix plus its settings. Persisted as `bottle.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bottle {
    pub id: String,
    pub name: String,
    /// Id of the engine this bottle uses; `None` until one is chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default)]
    pub windows_version: WindowsVersion,
    #[serde(default)]
    pub backend: GraphicsBackend,
    /// Bottle-wide graphics and performance switches (`[graphics]`).
    #[serde(default, skip_serializing_if = "GraphicsSettings::is_empty")]
    pub graphics: GraphicsSettings,
    /// Creation time, seconds since the Unix epoch.
    #[serde(default)]
    pub created_unix: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub apps: Vec<App>,
    /// Installed dependencies by verb name (`vcrun2022`, `dxvk`, ...).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub deps: BTreeMap<String, DepRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam: Option<SteamRecord>,
}

impl Bottle {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Result<Self> {
        let id = id.into();
        validate_bottle_id(&id)?;
        Ok(Self {
            id,
            name: name.into(),
            engine: None,
            windows_version: WindowsVersion::default(),
            backend: GraphicsBackend::default(),
            graphics: GraphicsSettings::default(),
            created_unix: 0,
            apps: Vec::new(),
            deps: BTreeMap::new(),
            steam: None,
        })
    }
}

/// Bottle ids become directory names, so keep them boring and path-safe.
pub fn validate_bottle_id(id: &str) -> Result<()> {
    let valid_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
    let starts_ok = id
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if id.len() <= 64 && starts_ok && id.chars().all(valid_char) {
        Ok(())
    } else {
        Err(Error::InvalidBottleId(id.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottle_ids_are_path_safe() {
        for ok in ["demo", "steam-2", "a_b", "0day"] {
            assert!(validate_bottle_id(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-x", "_x", "Demo", "a/b", "..", "a b", &"x".repeat(65)] {
            assert!(validate_bottle_id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn dll_overrides_are_joined_deterministically() {
        let mut p = LaunchProfile::default();
        assert_eq!(p.wine_dll_overrides(), None);
        p.dll_overrides.insert("dxgi".into(), "n".into());
        p.dll_overrides.insert("d3d11".into(), "n,b".into());
        assert_eq!(p.wine_dll_overrides().as_deref(), Some("d3d11=n,b;dxgi=n"));
    }

    #[test]
    fn explicit_env_overrides_dll_overrides_field() {
        let mut p = LaunchProfile::default();
        p.dll_overrides.insert("dxgi".into(), "n".into());
        p.env.insert("WINEDLLOVERRIDES".into(), "custom=d".into());
        p.env.insert("WINEMSYNC".into(), "1".into());
        let env = p.launch_env();
        assert_eq!(env["WINEDLLOVERRIDES"], "custom=d");
        assert_eq!(env["WINEMSYNC"], "1");
    }

    #[test]
    fn windows_versions_map_to_wine_names() {
        assert_eq!(WindowsVersion::Win11.wine_name(), "win11");
        assert_eq!(WindowsVersion::Win7.wine_name(), "win7");
        assert_eq!(WindowsVersion::default(), WindowsVersion::Win11);
    }

    #[test]
    fn graphics_settings_layer_and_round_trip() {
        let bottle = GraphicsSettings {
            metal_hud: Some(true),
            sync: Some(SyncMode::Msync),
            dxvk_hud: Some("fps".into()),
            ..Default::default()
        };
        let game = GraphicsSettings {
            metal_hud: Some(false),
            cpu_count: Some(4),
            ..Default::default()
        };
        let merged = game.or(&bottle);
        assert_eq!(merged.metal_hud, Some(false));
        assert_eq!(merged.sync, Some(SyncMode::Msync));
        assert_eq!(merged.dxvk_hud.as_deref(), Some("fps"));
        assert_eq!(merged.cpu_count, Some(4));
        assert!(GraphicsSettings::default().is_empty());

        let mut b = Bottle::new("g", "G").unwrap();
        b.graphics = bottle;
        let text = toml::to_string(&b).unwrap();
        assert!(text.contains("[graphics]"), "{text}");
        assert!(text.contains("sync = \"msync\""), "{text}");
        assert_eq!(toml::from_str::<Bottle>(&text).unwrap(), b);
        // Old bottle.toml files without the section still load.
        let old: Bottle = toml::from_str("id = \"x\"\nname = \"X\"\n").unwrap();
        assert!(old.graphics.is_empty());
    }

    #[test]
    fn rosetta_requirement_follows_translation() {
        let mut e = Engine {
            id: "wine-11".into(),
            wine_version: "11.16".into(),
            translation: Translation::Rosetta,
        };
        assert!(e.requires_rosetta());
        e.translation = Translation::Native;
        assert!(!e.requires_rosetta());
    }

    #[test]
    fn translation_names_match_serde_and_download_policy() {
        for t in [
            Translation::Rosetta,
            Translation::Native,
            Translation::External,
        ] {
            assert_eq!(serialized(t), t.name());
        }
        assert!(Translation::Rosetta.is_downloaded());
        assert!(Translation::Native.is_downloaded());
        assert!(!Translation::External.is_downloaded());
    }

    /// The string serde writes for a `Translation`.
    fn serialized(t: Translation) -> String {
        #[derive(Serialize)]
        struct W {
            t: Translation,
        }
        let text = toml::to_string(&W { t }).unwrap();
        text.trim()
            .trim_start_matches("t = ")
            .trim_matches('"')
            .to_owned()
    }
}
