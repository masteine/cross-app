//! Turning a bottle's (and a program's) graphics settings into what a launch
//! needs: which `wine` to run, environment variables and DLL overrides.
//!
//! - **Backend.** `dxvk` runs the engine's `wine` with `d3d10core`/`d3d11` set
//!   to the native DXVK DLLs that `cross deps install <bottle> dxvk` copied into
//!   the prefix. `d3dmetal` runs the `wine` of the engine's D3DMetal overlay
//!   (`cross engine d3dmetal enable`, see `cross_engine::d3dmetal`) with Apple's
//!   forwarders forced to builtin; without an overlay it falls back to Wine's
//!   own Direct3D (wined3d) and says so in [`GraphicsPlan::notes`].
//! - **Switches.** Each maps to one documented variable: `MTL_HUD_ENABLED` and
//!   `MTL_HUD_LOG_ENABLED` (Apple, "Monitoring your Metal app's graphics
//!   performance"), `DXVK_HUD` (DXVK README), `WINEESYNC`/`WINEMSYNC` and
//!   `WINENCPU`, the last three only when the engine was detected to read them
//!   (`cross_engine::features`); otherwise a note explains why nothing was set.
//! - **Retina mode** is a registry value of the Mac driver, see
//!   [`Runtime::set_retina_mode`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use cross_core::config::Config;
use cross_core::model::{Bottle, GraphicsBackend, GraphicsSettings, LaunchProfile, SyncMode};
use cross_core::paths::AppDirs;
use cross_engine::d3dmetal::Overlay;
use cross_engine::features::{self, EngineFeatures};
use cross_engine::gptk;
use cross_engine::manager::EngineManager;
use serde::Serialize;

use crate::registry::RegType;
use crate::runtime::Runtime;
use crate::{Error, Result};

/// DLLs DXVK replaces (the "builtin" DXVK build we install has no `dxgi`).
const DXVK_DLLS: [&str; 2] = ["d3d10core", "d3d11"];
/// DLLs that must stay builtin for D3DMetal: Apple's four forwarders plus
/// `d3d10core`, which a bottle-wide DXVK override would otherwise make native.
const D3DMETAL_DLLS: [&str; 5] = ["d3d10", "d3d10core", "d3d11", "d3d12", "dxgi"];
/// Upper bound for `cpu_count`; Windows programs see at most this many CPUs.
pub const MAX_CPU_COUNT: u32 = 256;

/// `HKCU\Software\Wine\Mac Driver`, read by `setup_options()` in Wine's
/// `dlls/winemac.drv/macdrv_main.c`.
const MAC_DRIVER_KEY: &str = r"HKCU\Software\Wine\Mac Driver";

/// What a launch will really use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectiveBackend {
    /// Apple's D3DMetal through the engine's overlay.
    D3dMetal,
    /// DXVK + MoltenVK from the prefix.
    Dxvk,
    /// Wine's own Direct3D (wined3d), the fallback when D3DMetal is not set up.
    Wined3d,
}

/// Everything about an engine that matters for graphics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineGraphics {
    pub engine_id: String,
    /// The engine's own `wine`.
    pub wine: PathBuf,
    /// The D3DMetal overlay, when one was built.
    pub d3dmetal: Option<Overlay>,
    /// The overlay was built from another GPTK import than the current one.
    pub d3dmetal_stale: bool,
    pub features: EngineFeatures,
}

impl EngineGraphics {
    /// Look at the engine `engine_id` and its D3DMetal overlay on disk.
    pub fn load(dirs: &AppDirs, engine_id: &str) -> Result<Self> {
        let engines = EngineManager::new(dirs);
        let engine = engines
            .get(engine_id)?
            .ok_or_else(|| Error::EngineNotInstalled(engine_id.to_owned()))?;
        let wine = engines.wine_path(&engine);
        let d3dmetal = engines.d3dmetal_overlay(&engine)?;
        let d3dmetal_stale = match &d3dmetal {
            Some(overlay) => {
                let gptk_dir = Config::load(dirs)?.gptk_dir(dirs);
                overlay.is_stale(gptk::inspect(&gptk_dir).as_ref())
            }
            None => false,
        };
        Ok(Self {
            engine_id: engine_id.to_owned(),
            features: features::probe(&wine),
            wine,
            d3dmetal,
            d3dmetal_stale,
        })
    }
}

/// The resolved graphics configuration of one launch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphicsPlan {
    pub requested: GraphicsBackend,
    pub effective: EffectiveBackend,
    /// The `wine` executable to start.
    pub wine: PathBuf,
    /// The settings after layering the program's over the bottle's.
    pub settings: GraphicsSettings,
    pub env: BTreeMap<String, String>,
    pub dll_overrides: BTreeMap<String, String>,
    /// Something that was asked for and not applied, or a real risk.
    pub warnings: Vec<String>,
    /// Background information (fallbacks of the bottle default, unchecked
    /// engines, stale overlays).
    pub notes: Vec<String>,
}

impl GraphicsPlan {
    /// Merge into `profile`. The profile's own env and DLL overrides win, so a
    /// hand-set value is never silently replaced.
    pub fn apply_to(&self, profile: &mut LaunchProfile) {
        for (k, v) in &self.env {
            profile.env.entry(k.clone()).or_insert_with(|| v.clone());
        }
        for (dll, mode) in &self.dll_overrides {
            profile
                .dll_overrides
                .entry(dll.clone())
                .or_insert_with(|| mode.clone());
        }
    }

    /// A runtime for `bottle_id` that starts this plan's `wine`.
    pub fn runtime(&self, dirs: &AppDirs, bottle_id: &str) -> Runtime {
        Runtime::new(dirs, bottle_id, &self.wine)
    }
}

/// A change to [`GraphicsSettings`]: `None` leaves a field alone,
/// `Some(None)` resets it to "inherit", `Some(Some(v))` sets it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SettingsUpdate {
    pub metal_hud: Option<Option<bool>>,
    pub metal_hud_log: Option<Option<bool>>,
    pub dxvk_hud: Option<Option<String>>,
    pub sync: Option<Option<SyncMode>>,
    pub cpu_count: Option<Option<u32>>,
}

impl SettingsUpdate {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Validate and apply to `settings`.
    pub fn apply(&self, settings: &mut GraphicsSettings) -> Result<()> {
        if let Some(Some(count)) = self.cpu_count {
            if !(1..=MAX_CPU_COUNT).contains(&count) {
                return Err(Error::InvalidInput(format!(
                    "cpu count must be between 1 and {MAX_CPU_COUNT}, got {count}"
                )));
            }
        }
        if let Some(v) = self.metal_hud {
            settings.metal_hud = v;
        }
        if let Some(v) = self.metal_hud_log {
            settings.metal_hud_log = v;
        }
        if let Some(v) = &self.dxvk_hud {
            settings.dxvk_hud = v.clone();
        }
        if let Some(v) = self.sync {
            settings.sync = v;
        }
        if let Some(v) = self.cpu_count {
            settings.cpu_count = v;
        }
        Ok(())
    }
}

/// Resolve the graphics plan for `bottle`, optionally for one program in it.
pub fn plan(
    bottle: &Bottle,
    engine: &EngineGraphics,
    program: Option<&LaunchProfile>,
) -> Result<GraphicsPlan> {
    let requested = program.and_then(|p| p.backend).unwrap_or(bottle.backend);
    let settings = program
        .map(|p| p.graphics.or(&bottle.graphics))
        .unwrap_or_else(|| bottle.graphics.clone());
    let explicit = program.and_then(|p| p.backend).is_some();
    let mut warnings = Vec::new();
    let mut notes = Vec::new();
    let mut env = BTreeMap::new();
    let mut dll_overrides = BTreeMap::new();

    let (effective, wine) = match requested {
        GraphicsBackend::Dxvk => {
            if !bottle.deps.contains_key("dxvk") {
                return Err(Error::InvalidInput(format!(
                    "the dxvk backend needs DXVK in the bottle: cross deps install {} dxvk",
                    bottle.id
                )));
            }
            for dll in DXVK_DLLS {
                dll_overrides.insert(dll.to_owned(), "native".to_owned());
            }
            (EffectiveBackend::Dxvk, engine.wine.clone())
        }
        GraphicsBackend::D3dMetal => match &engine.d3dmetal {
            Some(overlay) => {
                for dll in D3DMETAL_DLLS {
                    dll_overrides.insert(dll.to_owned(), "builtin".to_owned());
                }
                if overlay.record.forced {
                    warnings.push(format!(
                        "the D3DMetal overlay of {} was built with --force on an engine not known \
                         to run D3DMetal; programs using it may crash \
                         (https://github.com/frankea/Whisky/issues/163)",
                        engine.engine_id
                    ));
                }
                if engine.d3dmetal_stale {
                    notes.push(format!(
                        "the D3DMetal overlay predates the current GPTK import; rebuild it: \
                         cross engine d3dmetal enable {}",
                        engine.engine_id
                    ));
                }
                (EffectiveBackend::D3dMetal, overlay.wine.clone())
            }
            None => {
                let reason = format!(
                    "D3DMetal is not set up for engine {} (see `cross engine d3dmetal status {}`)",
                    engine.engine_id, engine.engine_id
                );
                if explicit {
                    // Asked for by this program: undo a bottle-wide DXVK override.
                    for dll in DXVK_DLLS {
                        dll_overrides.insert(dll.to_owned(), "builtin".to_owned());
                    }
                    warnings.push(format!(
                        "{reason}; using Wine's built-in Direct3D (wined3d)"
                    ));
                    (EffectiveBackend::Wined3d, engine.wine.clone())
                } else if bottle.deps.contains_key("dxvk") {
                    // The bottle default: leave the prefix's own DLL overrides alone.
                    notes.push(format!("{reason}; the bottle's DXVK DLL overrides apply"));
                    (EffectiveBackend::Dxvk, engine.wine.clone())
                } else {
                    notes.push(format!(
                        "{reason}; using Wine's built-in Direct3D (wined3d)"
                    ));
                    (EffectiveBackend::Wined3d, engine.wine.clone())
                }
            }
        },
    };

    let hud_log = settings.metal_hud_log == Some(true);
    if settings.metal_hud == Some(true) || hud_log {
        env.insert("MTL_HUD_ENABLED".to_owned(), "1".to_owned());
    }
    if hud_log {
        // Apple: per-frame statistics once per second, needs MTL_HUD_ENABLED=1.
        env.insert("MTL_HUD_LOG_ENABLED".to_owned(), "1".to_owned());
    }

    if let Some(spec) = settings.dxvk_hud.as_deref().filter(|s| !s.is_empty()) {
        if effective == EffectiveBackend::Dxvk {
            env.insert("DXVK_HUD".to_owned(), spec.to_owned());
        } else {
            let message = "the DXVK HUD only works with the dxvk backend; not set".to_owned();
            // Inherited from the bottle by a program on another backend: expected.
            let inherited = program.is_some_and(|p| p.graphics.dxvk_hud.is_none());
            if inherited {
                notes.push(message);
            } else {
                warnings.push(message);
            }
        }
    }

    let features = engine.features;
    let unchecked = !features.inspected;
    match settings.sync {
        None | Some(SyncMode::None) => {}
        Some(SyncMode::Esync) if features.esync || unchecked => {
            env.insert("WINEESYNC".to_owned(), "1".to_owned());
        }
        Some(SyncMode::Msync) if features.msync || unchecked => {
            env.insert("WINEMSYNC".to_owned(), "1".to_owned());
            // Whisky sets both: D3DMetal checks for esync and changes its
            // behaviour (WhiskyKit BottleSettings.swift, "values hardcoded in
            // libd3dshared.dylib"). Unverified here.
            env.insert("WINEESYNC".to_owned(), "1".to_owned());
        }
        Some(mode) => warnings.push(format!(
            "engine {} has no {} support (the build does not read {}); not set",
            engine.engine_id,
            if mode == SyncMode::Esync {
                "esync"
            } else {
                "msync"
            },
            if mode == SyncMode::Esync {
                "WINEESYNC"
            } else {
                "WINEMSYNC"
            },
        )),
    }

    if let Some(count) = settings.cpu_count {
        if !(1..=MAX_CPU_COUNT).contains(&count) {
            return Err(Error::InvalidInput(format!(
                "cpu count must be between 1 and {MAX_CPU_COUNT}, got {count}"
            )));
        }
        if features.cpu_count_override || unchecked {
            env.insert("WINENCPU".to_owned(), count.to_string());
        } else {
            warnings.push(format!(
                "engine {} cannot limit the CPU count (it does not read WINENCPU); not set",
                engine.engine_id
            ));
        }
    }
    if unchecked && (settings.sync.is_some() || settings.cpu_count.is_some()) {
        notes.push(format!(
            "could not inspect engine {}; sync/CPU settings were set without checking support",
            engine.engine_id
        ));
    }

    tracing::info!(
        bottle = %bottle.id,
        engine = %engine.engine_id,
        ?requested,
        ?effective,
        "graphics plan"
    );
    Ok(GraphicsPlan {
        requested,
        effective,
        wine,
        settings,
        env,
        dll_overrides,
        warnings,
        notes,
    })
}

/// Load the bottle's engine and resolve the plan in one go.
pub fn plan_for(
    dirs: &AppDirs,
    bottle: &Bottle,
    program: Option<&LaunchProfile>,
) -> Result<GraphicsPlan> {
    let engine_id = bottle
        .engine
        .as_deref()
        .ok_or_else(|| Error::BottleHasNoEngine {
            bottle: bottle.id.clone(),
        })?;
    plan(bottle, &EngineGraphics::load(dirs, engine_id)?, program)
}

impl Runtime {
    /// Turn the Mac driver's Retina mode on or off for the whole bottle
    /// (`HKCU\Software\Wine\Mac Driver\RetinaMode`, `y`/`n`). Wine reads it
    /// from the bottle-wide key only, never per program, when the Mac driver
    /// starts, so running programs keep the old mode until restarted.
    pub fn set_retina_mode(&self, on: bool) -> Result<()> {
        self.reg_set(
            MAC_DRIVER_KEY,
            "RetinaMode",
            RegType::Sz,
            if on { "y" } else { "n" },
        )
    }

    /// Current Retina mode; `None` when never set (Wine's default is off).
    pub fn retina_mode(&self) -> Result<Option<bool>> {
        Ok(self
            .reg_get(MAC_DRIVER_KEY, "RetinaMode")?
            .map(|v| is_option_true(&v.data)))
    }
}

/// Wine's `IS_OPTION_TRUE`: the first character is one of `yYtT1`.
fn is_option_true(data: &str) -> bool {
    matches!(data.chars().next(), Some('y' | 'Y' | 't' | 'T' | '1'))
}

#[cfg(test)]
mod tests {
    use cross_core::model::{DepRecord, DepSource};
    use cross_engine::d3dmetal::OverlayRecord;

    use std::path::Path;

    use super::*;
    use crate::runtime::test_support::fake_wine;

    fn bottle() -> Bottle {
        let mut b = Bottle::new("games", "Games").unwrap();
        b.engine = Some("eng".into());
        b
    }

    fn with_dxvk(mut b: Bottle) -> Bottle {
        b.deps.insert(
            "dxvk".into(),
            DepRecord {
                installed_unix: 1,
                source: DepSource::Catalog,
            },
        );
        b
    }

    fn engine(overlay: bool, features: EngineFeatures) -> EngineGraphics {
        EngineGraphics {
            engine_id: "eng".into(),
            wine: "/e/bin/wine".into(),
            d3dmetal: overlay.then(|| Overlay {
                dir: "/e/d3dmetal".into(),
                wine: "/e/d3dmetal/bin/wine".into(),
                record: OverlayRecord {
                    gptk_version: Some("4.0".into()),
                    forced: false,
                    created_unix: 0,
                },
            }),
            d3dmetal_stale: false,
            features,
        }
    }

    fn upstream() -> EngineFeatures {
        EngineFeatures {
            inspected: true,
            retina_mode: true,
            ..Default::default()
        }
    }

    fn patched() -> EngineFeatures {
        EngineFeatures {
            inspected: true,
            esync: true,
            msync: true,
            cpu_count_override: true,
            retina_mode: true,
        }
    }

    #[test]
    fn d3dmetal_uses_the_overlay_when_present() {
        let p = plan(&bottle(), &engine(true, upstream()), None).unwrap();
        assert_eq!(p.effective, EffectiveBackend::D3dMetal);
        assert_eq!(p.wine, Path::new("/e/d3dmetal/bin/wine"));
        for dll in ["d3d10", "d3d10core", "d3d11", "d3d12", "dxgi"] {
            assert_eq!(p.dll_overrides[dll], "builtin", "{dll}");
        }
        assert!(p.notes.is_empty() && p.warnings.is_empty(), "{p:?}");
    }

    #[test]
    fn d3dmetal_falls_back_without_an_overlay() {
        // Bottle default: nothing forced, just a note.
        let p = plan(&bottle(), &engine(false, upstream()), None).unwrap();
        assert_eq!(p.effective, EffectiveBackend::Wined3d);
        assert_eq!(p.wine, Path::new("/e/bin/wine"));
        assert!(p.dll_overrides.is_empty());
        assert!(p.warnings.is_empty());
        assert!(p.notes[0].contains("wined3d"), "{:?}", p.notes);

        // Bottle default with DXVK installed: the prefix's overrides decide.
        let p = plan(&with_dxvk(bottle()), &engine(false, upstream()), None).unwrap();
        assert_eq!(p.effective, EffectiveBackend::Dxvk);
        assert!(p.dll_overrides.is_empty());

        // Asked for by the program: DXVK is undone and the user is warned.
        let game = LaunchProfile {
            backend: Some(GraphicsBackend::D3dMetal),
            ..Default::default()
        };
        let p = plan(
            &with_dxvk(bottle()),
            &engine(false, upstream()),
            Some(&game),
        )
        .unwrap();
        assert_eq!(p.effective, EffectiveBackend::Wined3d);
        assert_eq!(p.dll_overrides["d3d11"], "builtin");
        assert_eq!(p.dll_overrides["d3d10core"], "builtin");
        assert!(!p.dll_overrides.contains_key("dxgi"));
        assert!(p.warnings[0].contains("wined3d"), "{:?}", p.warnings);
    }

    #[test]
    fn forced_and_stale_overlays_are_flagged() {
        let mut e = engine(true, upstream());
        e.d3dmetal.as_mut().unwrap().record.forced = true;
        e.d3dmetal_stale = true;
        let p = plan(&bottle(), &e, None).unwrap();
        assert!(p.warnings[0].contains("issues/163"), "{:?}", p.warnings);
        assert!(p.notes[0].contains("cross engine d3dmetal enable eng"));
    }

    #[test]
    fn program_backend_overrides_the_bottle() {
        let mut b = with_dxvk(bottle());
        b.backend = GraphicsBackend::D3dMetal;
        let game = LaunchProfile {
            backend: Some(GraphicsBackend::Dxvk),
            ..Default::default()
        };
        let p = plan(&b, &engine(true, upstream()), Some(&game)).unwrap();
        assert_eq!(p.effective, EffectiveBackend::Dxvk);
        assert_eq!(p.wine, Path::new("/e/bin/wine"));
        assert_eq!(p.dll_overrides["d3d11"], "native");
        assert_eq!(p.dll_overrides["d3d10core"], "native");

        let err = plan(&bottle(), &engine(true, upstream()), Some(&game)).unwrap_err();
        assert!(err.to_string().contains("cross deps install games dxvk"));
    }

    #[test]
    fn huds_follow_the_backend() {
        let mut b = with_dxvk(bottle());
        b.backend = GraphicsBackend::Dxvk;
        b.graphics.metal_hud_log = Some(true);
        b.graphics.dxvk_hud = Some("fps,frametimes".into());
        let p = plan(&b, &engine(false, upstream()), None).unwrap();
        assert_eq!(p.env["MTL_HUD_ENABLED"], "1");
        assert_eq!(p.env["MTL_HUD_LOG_ENABLED"], "1");
        assert_eq!(p.env["DXVK_HUD"], "fps,frametimes");

        // A game turns the HUD log off and runs on D3DMetal: no DXVK HUD there.
        let game = LaunchProfile {
            backend: Some(GraphicsBackend::D3dMetal),
            graphics: GraphicsSettings {
                metal_hud_log: Some(false),
                metal_hud: Some(true),
                ..Default::default()
            },
            ..Default::default()
        };
        let p = plan(&b, &engine(true, upstream()), Some(&game)).unwrap();
        assert_eq!(p.env.get("MTL_HUD_LOG_ENABLED"), None);
        assert_eq!(p.env["MTL_HUD_ENABLED"], "1");
        assert_eq!(p.env.get("DXVK_HUD"), None);
        assert!(p.notes.iter().any(|n| n.contains("DXVK HUD")), "{p:?}");
        assert!(!p.warnings.iter().any(|n| n.contains("DXVK HUD")), "{p:?}");

        // Set on the bottle itself while the bottle runs D3DMetal: a warning.
        b.backend = GraphicsBackend::D3dMetal;
        let p = plan(&b, &engine(true, upstream()), None).unwrap();
        assert!(p.warnings.iter().any(|n| n.contains("DXVK HUD")), "{p:?}");
    }

    #[test]
    fn sync_and_cpu_count_need_engine_support() {
        let mut b = bottle();
        b.graphics.sync = Some(SyncMode::Msync);
        b.graphics.cpu_count = Some(4);

        let p = plan(&b, &engine(false, patched()), None).unwrap();
        assert_eq!(p.env["WINEMSYNC"], "1");
        assert_eq!(p.env["WINEESYNC"], "1");
        assert_eq!(p.env["WINENCPU"], "4");

        let p = plan(&b, &engine(false, upstream()), None).unwrap();
        assert!(!p.env.contains_key("WINEMSYNC"));
        assert!(!p.env.contains_key("WINENCPU"));
        assert!(p.warnings.iter().any(|n| n.contains("no msync support")));
        assert!(p.warnings.iter().any(|n| n.contains("WINENCPU")));

        b.graphics.sync = Some(SyncMode::Esync);
        let p = plan(&b, &engine(false, patched()), None).unwrap();
        assert_eq!(p.env["WINEESYNC"], "1");
        assert!(!p.env.contains_key("WINEMSYNC"));

        // An engine we could not inspect: set, but say it was not checked.
        let p = plan(&b, &engine(false, EngineFeatures::default()), None).unwrap();
        assert_eq!(p.env["WINEESYNC"], "1");
        assert!(p.notes.iter().any(|n| n.contains("could not inspect")));

        b.graphics.cpu_count = Some(0);
        assert!(plan(&b, &engine(false, patched()), None).is_err());
    }

    #[test]
    fn plan_merges_without_overwriting_explicit_values() {
        let mut b = bottle();
        b.graphics.metal_hud = Some(true);
        let p = plan(&b, &engine(true, upstream()), None).unwrap();
        let mut profile = LaunchProfile::default();
        profile.env.insert("MTL_HUD_ENABLED".into(), "0".into());
        profile.dll_overrides.insert("dxgi".into(), "native".into());
        p.apply_to(&mut profile);
        assert_eq!(profile.env["MTL_HUD_ENABLED"], "0");
        assert_eq!(profile.dll_overrides["dxgi"], "native");
        assert_eq!(profile.dll_overrides["d3d12"], "builtin");
    }

    #[test]
    fn settings_updates_set_reset_and_validate() {
        let mut s = GraphicsSettings {
            metal_hud: Some(true),
            sync: Some(SyncMode::Esync),
            ..Default::default()
        };
        SettingsUpdate {
            metal_hud: Some(None),
            dxvk_hud: Some(Some("fps".into())),
            cpu_count: Some(Some(2)),
            ..Default::default()
        }
        .apply(&mut s)
        .unwrap();
        assert_eq!(s.metal_hud, None);
        assert_eq!(s.sync, Some(SyncMode::Esync));
        assert_eq!(s.dxvk_hud.as_deref(), Some("fps"));
        assert_eq!(s.cpu_count, Some(2));
        let bad = SettingsUpdate {
            cpu_count: Some(Some(999)),
            ..Default::default()
        };
        assert!(bad.apply(&mut s).is_err());
        assert_eq!(s.cpu_count, Some(2));
        assert!(SettingsUpdate::default().is_empty());
    }

    #[test]
    fn retina_mode_is_a_mac_driver_string() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = fake_wine(tmp.path());
        let rt = Runtime::new(&AppDirs::with_root(tmp.path()), "demo", &fake.wine);
        assert_eq!(rt.retina_mode().unwrap(), None);
        rt.set_retina_mode(true).unwrap();
        assert_eq!(rt.retina_mode().unwrap(), Some(true));
        assert!(fake.calls().contains(
            r"ARGS:reg add HKCU\Software\Wine\Mac Driver /v RetinaMode /t REG_SZ /d y /f"
        ));
        rt.set_retina_mode(false).unwrap();
        assert_eq!(rt.retina_mode().unwrap(), Some(false));
        assert!(is_option_true("True") && is_option_true("1") && !is_option_true("off"));
    }

    #[test]
    fn engine_graphics_loads_from_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        assert!(matches!(
            EngineGraphics::load(&dirs, "nope").unwrap_err(),
            Error::EngineNotInstalled(_)
        ));
        let err = plan_for(&dirs, &Bottle::new("x", "X").unwrap(), None).unwrap_err();
        assert!(matches!(err, Error::BottleHasNoEngine { .. }), "{err}");
    }
}
