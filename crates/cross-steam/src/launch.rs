//! Starting Steam and Steam games, with per-game settings.
//!
//! Games are started the way Steam itself does it: `steam.exe -applaunch <appid>
//! [game args]`. Per-game environment (backend, DLL overrides, env vars) is set on
//! that `steam.exe`, so it reaches the game only when this launch also starts
//! Steam. If Steam is already running in the bottle, the running client starts
//! the game with its own environment; callers are told so.

use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};

use cross_bottle::graphics::{self, GraphicsPlan, SettingsUpdate};
use cross_bottle::launch::{self, Flags, LaunchRequest};
use cross_bottle::Runtime;
use cross_core::config::BottleStore;
use cross_core::model::{App, Bottle, GraphicsBackend, LaunchProfile};
use cross_core::paths::AppDirs;

use crate::install::is_installed;
use crate::patches::SteamPatches;
use crate::{Error, Result, STEAM_EXE_WIN};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameLaunch {
    /// Steam was already running, so per-game settings could not be applied.
    pub steam_was_running: bool,
    /// The game has its own settings in this bottle.
    pub has_profile: bool,
    /// Where Steam's output goes.
    pub log: PathBuf,
    /// The graphics configuration `steam.exe` was started with.
    pub graphics: GraphicsPlan,
}

/// Per-game settings stored in `bottle.toml` (an `App` with this `steam_appid`).
pub fn game_profile(bottle: &Bottle, appid: u32) -> Option<&LaunchProfile> {
    bottle
        .apps
        .iter()
        .find(|a| a.steam_appid == Some(appid))
        .map(|a| &a.profile)
}

/// Everything `steam.exe` is started with: the patch file's flags, then the
/// game's env/overrides/args, then what the graphics `plan` adds (explicit
/// values win), then `-applaunch <appid> <game args>`.
pub fn steam_request(
    patches: &SteamPatches,
    bottle: &Bottle,
    appid: Option<u32>,
    extra_args: &[String],
    plan: &GraphicsPlan,
) -> LaunchRequest {
    let mut profile = patches.launch_profile();
    let mut args = extra_args.to_vec();

    if let Some(appid) = appid {
        args.extend(["-applaunch".to_owned(), appid.to_string()]);
        if let Some(game) = game_profile(bottle, appid) {
            profile.env.extend(game.env.clone());
            profile.dll_overrides.extend(game.dll_overrides.clone());
            args.extend(game.args.iter().cloned());
        }
    }
    plan.apply_to(&mut profile);

    LaunchRequest {
        target: STEAM_EXE_WIN.to_owned(),
        args,
        working_dir: None,
        profile,
        flags: Flags::default(),
    }
}

/// Check that `backend` can be used for a game in `bottle` (DXVK must be
/// installed in it).
pub fn validate_backend(bottle: &Bottle, backend: GraphicsBackend) -> Result<()> {
    if backend == GraphicsBackend::Dxvk && !bottle.deps.contains_key("dxvk") {
        return Err(Error::InvalidInput(format!(
            "the dxvk backend needs DXVK in the bottle: cross deps install {} dxvk",
            bottle.id
        )));
    }
    Ok(())
}

/// Open the bottle, check Steam is there and bring its registry patches up to
/// date if `steam-patches.toml` changed since they were applied.
fn prepare(dirs: &AppDirs, bottle_id: &str) -> Result<(Bottle, Runtime, SteamPatches)> {
    let (mut bottle, runtime) = Runtime::open(dirs, bottle_id)?;
    if !is_installed(runtime.prefix()) {
        return Err(Error::NotInstalled(bottle_id.to_owned()));
    }
    let (patches, _) = SteamPatches::load(dirs)?;
    let applied = bottle.steam.as_ref().map(|s| s.patches_revision);
    if applied.is_some_and(|rev| rev < patches.revision) {
        tracing::info!(from = ?applied, to = patches.revision, "re-applying updated Steam patches");
        patches.apply_registry(&runtime)?;
        if let Some(record) = bottle.steam.as_mut() {
            record.patches_revision = patches.revision;
        }
        BottleStore::new(dirs).save(&bottle)?;
    }
    Ok((bottle, runtime, patches))
}

/// Run the Steam client in the foreground until it exits.
pub fn run_client(dirs: &AppDirs, bottle_id: &str, extra_args: &[String]) -> Result<ExitStatus> {
    let (bottle, _, patches) = prepare(dirs, bottle_id)?;
    let plan = graphics::plan_for(dirs, &bottle, None)?;
    for warning in &plan.warnings {
        tracing::warn!(bottle = bottle_id, "{warning}");
    }
    let request = steam_request(&patches, &bottle, None, extra_args, &plan);
    Ok(launch::run(&plan.runtime(dirs, bottle_id), &request)?)
}

/// Start a game through Steam and return right away; Steam keeps running in the
/// background with its output in the bottle's `logs/steam.log`.
pub fn launch_game(dirs: &AppDirs, bottle_id: &str, appid: u32) -> Result<GameLaunch> {
    let (bottle, runtime, patches) = prepare(dirs, bottle_id)?;
    let steam_was_running = runtime
        .processes()?
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case("steam.exe"));
    let game = game_profile(&bottle, appid);
    let has_profile = game.is_some_and(|p| *p != LaunchProfile::default());

    let plan = graphics::plan_for(dirs, &bottle, game)?;
    let request = steam_request(&patches, &bottle, Some(appid), &[], &plan);
    let logs = dirs.bottle_logs_dir(bottle_id);
    std::fs::create_dir_all(&logs).map_err(|e| Error::io(&logs, e))?;
    let log = logs.join("steam.log");
    let open = || {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
            .map_err(|e| Error::io(&log, e))
    };

    let mut cmd = launch::build_command(&plan.runtime(dirs, bottle_id), &request)?;
    cmd.stdin(Stdio::null()).stdout(open()?).stderr(open()?);
    tracing::info!(
        bottle = bottle_id,
        appid,
        steam_was_running,
        backend = ?plan.effective,
        "launching game via Steam"
    );
    cmd.spawn()
        .map_err(|e| Error::Setup(format!("could not start steam.exe: {e}")))?;
    Ok(GameLaunch {
        steam_was_running,
        has_profile,
        log,
        graphics: plan,
    })
}

/// Changes to a game's settings; `None` leaves a field as it is.
#[derive(Debug, Clone, Default)]
pub struct ProfileUpdate {
    pub name: Option<String>,
    pub backend: Option<GraphicsBackend>,
    pub env: Vec<(String, String)>,
    pub args: Option<Vec<String>>,
    /// Graphics switches for this game (HUDs, sync, CPU count).
    pub graphics: SettingsUpdate,
    /// Drop all settings for the game.
    pub clear: bool,
}

/// Create, change or remove a game's settings in `bottle.toml`.
pub fn update_game_profile(
    dirs: &AppDirs,
    bottle_id: &str,
    appid: u32,
    update: ProfileUpdate,
) -> Result<Option<App>> {
    let store = BottleStore::new(dirs);
    let mut bottle = store.load(bottle_id)?;
    if update.clear {
        bottle.apps.retain(|a| a.steam_appid != Some(appid));
        store.save(&bottle)?;
        return Ok(None);
    }
    if let Some(backend) = update.backend {
        validate_backend(&bottle, backend)?; // before anything is saved
    }
    let index = match bottle
        .apps
        .iter()
        .position(|a| a.steam_appid == Some(appid))
    {
        Some(i) => i,
        None => {
            bottle.apps.push(App {
                name: format!("Steam app {appid}"),
                path: STEAM_EXE_WIN.into(),
                steam_appid: Some(appid),
                profile: LaunchProfile::default(),
            });
            bottle.apps.len() - 1
        }
    };
    let app = &mut bottle.apps[index];
    if let Some(name) = update.name {
        app.name = name;
    }
    if update.backend.is_some() {
        app.profile.backend = update.backend;
    }
    for (k, v) in update.env {
        if v.is_empty() {
            app.profile.env.remove(&k);
        } else {
            app.profile.env.insert(k, v);
        }
    }
    if let Some(args) = update.args {
        app.profile.args = args;
    }
    update.graphics.apply(&mut app.profile.graphics)?;
    let app = app.clone();
    store.save(&bottle)?;
    Ok(Some(app))
}

#[cfg(test)]
mod tests {
    use cross_bottle::graphics::{plan, EffectiveBackend, EngineGraphics};
    use cross_core::model::{DepRecord, DepSource, GraphicsSettings, SyncMode};
    use cross_engine::d3dmetal::{Overlay, OverlayRecord};
    use cross_engine::features::EngineFeatures;

    use super::*;

    fn bottle() -> Bottle {
        let mut b = Bottle::new("games", "Games").unwrap();
        b.engine = Some("eng".into());
        b
    }

    /// An engine with a D3DMetal overlay (synthetic; nothing runs).
    fn engine(overlay: bool) -> EngineGraphics {
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
            features: EngineFeatures {
                inspected: true,
                msync: true,
                ..Default::default()
            },
        }
    }

    fn request(b: &Bottle, appid: Option<u32>, overlay: bool) -> (LaunchRequest, GraphicsPlan) {
        let game = appid.and_then(|id| game_profile(b, id));
        let plan = plan(b, &engine(overlay), game).unwrap();
        (
            steam_request(&SteamPatches::builtin(), b, appid, &[], &plan),
            plan,
        )
    }

    #[test]
    fn plain_client_launch_uses_patch_flags() {
        let patches = SteamPatches::builtin();
        let plan = plan(&bottle(), &engine(false), None).unwrap();
        let req = steam_request(&patches, &bottle(), None, &["-console".into()], &plan);
        assert_eq!(req.target, STEAM_EXE_WIN);
        assert_eq!(req.args, ["-console"]);
        assert_eq!(req.profile.args, patches.launch.args);
    }

    #[test]
    fn game_launch_applies_the_game_profile() {
        let mut b = bottle();
        b.deps.insert(
            "dxvk".into(),
            DepRecord {
                installed_unix: 1,
                source: DepSource::Catalog,
            },
        );
        let mut profile = LaunchProfile {
            backend: Some(GraphicsBackend::Dxvk),
            args: vec!["-windowed".into()],
            ..Default::default()
        };
        profile.env.insert("DXVK_HUD".into(), "fps".into());
        profile
            .dll_overrides
            .insert("xinput1_3".into(), "native".into());
        b.apps.push(App {
            name: "HL2".into(),
            path: STEAM_EXE_WIN.into(),
            steam_appid: Some(220),
            profile,
        });

        let (req, plan) = request(&b, Some(220), true);
        assert_eq!(plan.effective, EffectiveBackend::Dxvk);
        assert_eq!(plan.wine, std::path::Path::new("/e/bin/wine"));
        assert_eq!(req.args, ["-applaunch", "220", "-windowed"]);
        assert_eq!(req.profile.env["DXVK_HUD"], "fps");
        assert_eq!(req.profile.dll_overrides["d3d11"], "native");
        assert_eq!(req.profile.dll_overrides["d3d10core"], "native");
        assert_eq!(req.profile.dll_overrides["xinput1_3"], "native");

        // Another game follows the bottle (D3DMetal by default): no extras
        // without an overlay, Apple's forwarders through the overlay with one.
        let (other, _) = request(&b, Some(440), false);
        assert_eq!(other.args, ["-applaunch", "440"]);
        assert!(!other.profile.dll_overrides.contains_key("d3d11"));
        let (other, plan) = request(&b, Some(440), true);
        assert_eq!(plan.effective, EffectiveBackend::D3dMetal);
        assert_eq!(plan.wine, std::path::Path::new("/e/d3dmetal/bin/wine"));
        assert_eq!(other.profile.dll_overrides["d3d12"], "builtin");
        assert_eq!(other.profile.dll_overrides["dxgi"], "builtin");
    }

    #[test]
    fn d3dmetal_game_gets_the_overlay_and_its_switches() {
        let mut b = bottle();
        b.graphics.sync = Some(SyncMode::Msync);
        b.apps.push(App {
            name: "BG3".into(),
            path: STEAM_EXE_WIN.into(),
            steam_appid: Some(1086940),
            profile: LaunchProfile {
                backend: Some(GraphicsBackend::D3dMetal),
                graphics: GraphicsSettings {
                    metal_hud: Some(true),
                    ..Default::default()
                },
                ..Default::default()
            },
        });
        let (req, plan) = request(&b, Some(1086940), true);
        assert_eq!(plan.effective, EffectiveBackend::D3dMetal);
        assert_eq!(req.profile.env["MTL_HUD_ENABLED"], "1");
        assert_eq!(req.profile.env["WINEMSYNC"], "1");
        assert_eq!(req.profile.dll_overrides["d3d11"], "builtin");
        // The Steam patch flags survive.
        assert_eq!(req.profile.args, SteamPatches::builtin().launch.args);

        // Without an overlay the game is told it runs on wined3d.
        let (req, plan) = request(&b, Some(1086940), false);
        assert_eq!(plan.effective, EffectiveBackend::Wined3d);
        assert_eq!(req.profile.dll_overrides["d3d11"], "builtin");
        assert!(!plan.warnings.is_empty());
    }

    #[test]
    fn dxvk_backend_requires_dxvk_in_the_bottle() {
        let err = validate_backend(&bottle(), GraphicsBackend::Dxvk)
            .unwrap_err()
            .to_string();
        assert!(err.contains("cross deps install games dxvk"), "{err}");
        validate_backend(&bottle(), GraphicsBackend::D3dMetal).unwrap();
    }

    #[test]
    fn profiles_can_be_created_changed_and_cleared() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        BottleStore::new(&dirs).save(&bottle()).unwrap();

        let app = update_game_profile(
            &dirs,
            "games",
            220,
            ProfileUpdate {
                name: Some("Half-Life 2".into()),
                backend: Some(GraphicsBackend::D3dMetal),
                env: vec![("A".into(), "1".into()), ("B".into(), "2".into())],
                args: Some(vec!["-novid".into()]),
                graphics: SettingsUpdate {
                    metal_hud_log: Some(Some(true)),
                    ..Default::default()
                },
                clear: false,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(app.name, "Half-Life 2");
        assert_eq!(app.profile.backend, Some(GraphicsBackend::D3dMetal));
        assert_eq!(app.profile.graphics.metal_hud_log, Some(true));

        // Empty value removes a variable; untouched fields stay.
        let app = update_game_profile(
            &dirs,
            "games",
            220,
            ProfileUpdate {
                env: vec![("A".into(), String::new())],
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(!app.profile.env.contains_key("A"));
        assert_eq!(app.profile.env["B"], "2");
        assert_eq!(app.profile.args, ["-novid"]);

        let saved = BottleStore::new(&dirs).load("games").unwrap();
        assert_eq!(saved.apps.len(), 1);

        assert!(update_game_profile(
            &dirs,
            "games",
            220,
            ProfileUpdate {
                clear: true,
                ..Default::default()
            }
        )
        .unwrap()
        .is_none());
        assert!(BottleStore::new(&dirs)
            .load("games")
            .unwrap()
            .apps
            .is_empty());

        let err = update_game_profile(
            &dirs,
            "games",
            220,
            ProfileUpdate {
                backend: Some(GraphicsBackend::Dxvk),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "{err}");
        assert!(BottleStore::new(&dirs)
            .load("games")
            .unwrap()
            .apps
            .is_empty());
    }
}
