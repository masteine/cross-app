//! `#[tauri::command]` wrappers around [`crate::api`]. Every command runs its
//! work off the main thread; long ones go through [`tasks::run`] for progress.

use std::path::PathBuf;

use cross_core::model::WindowsVersion;
use cross_core::system::GptkInfo;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use super::tasks::{self, notify};
use super::Gui;
use crate::api::apps::{self, AppView, Spawned, PROGRAM_EXTENSIONS};
use crate::api::bottles::{self, BottleDetail, BottleSummary, DepOutcome, NewBottle, StopResult};
use crate::api::engines::{self, EngineRow, EnginesView};
use crate::api::library::{self, GameLaunched, Library};
use crate::api::logs::{self, LogChunk, LogFile};
use crate::api::system::{self, RosettaOutcome, SystemStatus};
use crate::{Error, Result};

// --- system & first-run wizard -------------------------------------------------

#[tauri::command]
pub async fn system_status(gui: State<'_, Gui>) -> Result<SystemStatus> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || system::status(&dirs)).await
}

#[tauri::command]
pub async fn install_rosetta(app: AppHandle, task: String) -> Result<RosettaOutcome> {
    tasks::run(app, task, "Installing Rosetta 2".into(), |_| {
        system::install_rosetta()
    })
    .await
}

// --- engines -------------------------------------------------------------------

#[tauri::command]
pub async fn list_engines(gui: State<'_, Gui>) -> Result<EnginesView> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || engines::list(&dirs)).await
}

#[tauri::command]
pub async fn install_engine(
    app: AppHandle,
    gui: State<'_, Gui>,
    task: String,
    id: String,
    dxvk: bool,
) -> Result<EngineRow> {
    let dirs = gui.dirs.clone();
    let title = format!("Installing {id}");
    tasks::run(app, task, title, move |progress| {
        engines::install(&dirs, &id, dxvk, progress)
    })
    .await
}

#[tauri::command]
pub async fn remove_engine(gui: State<'_, Gui>, id: String) -> Result<()> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || engines::remove(&dirs, &id)).await
}

#[tauri::command]
pub async fn set_default_engine(gui: State<'_, Gui>, id: String) -> Result<()> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || engines::set_default(&dirs, &id)).await
}

#[tauri::command]
pub async fn import_gptk(
    app: AppHandle,
    gui: State<'_, Gui>,
    task: String,
    path: PathBuf,
    version: Option<String>,
) -> Result<GptkInfo> {
    let dirs = gui.dirs.clone();
    tasks::run(
        app,
        task,
        "Importing Game Porting Toolkit".into(),
        move |_| engines::import_gptk(&dirs, &path, version.as_deref()),
    )
    .await
}

// --- bottles -------------------------------------------------------------------

#[tauri::command]
pub async fn list_bottles(gui: State<'_, Gui>) -> Result<Vec<BottleSummary>> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || bottles::list(&dirs)).await
}

#[tauri::command]
pub async fn bottle_detail(gui: State<'_, Gui>, id: String) -> Result<BottleDetail> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || bottles::detail(&dirs, &id)).await
}

#[tauri::command]
pub async fn create_bottle(
    app: AppHandle,
    gui: State<'_, Gui>,
    task: String,
    options: NewBottle,
) -> Result<BottleSummary> {
    let dirs = gui.dirs.clone();
    let title = format!("Creating bottle {}", options.id);
    tasks::run(app, task, title, move |_| bottles::create(&dirs, options)).await
}

#[tauri::command]
pub async fn delete_bottle(gui: State<'_, Gui>, id: String) -> Result<()> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || bottles::delete(&dirs, &id)).await
}

#[tauri::command]
pub async fn set_windows_version(
    gui: State<'_, Gui>,
    id: String,
    windows: WindowsVersion,
) -> Result<()> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || bottles::set_windows(&dirs, &id, windows)).await
}

#[tauri::command]
pub async fn set_bottle_engine(gui: State<'_, Gui>, id: String, engine: String) -> Result<()> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || bottles::set_engine(&dirs, &id, &engine)).await
}

#[tauri::command]
pub async fn bottle_processes(
    gui: State<'_, Gui>,
    id: String,
) -> Result<Vec<cross_bottle::processes::WinProcess>> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || bottles::processes(&dirs, &id)).await
}

#[tauri::command]
pub async fn stop_bottle(gui: State<'_, Gui>, id: String, force: bool) -> Result<StopResult> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || bottles::stop(&dirs, &id, force)).await
}

#[tauri::command]
pub async fn install_deps(
    app: AppHandle,
    gui: State<'_, Gui>,
    task: String,
    id: String,
    verbs: Vec<String>,
    force: bool,
    winetricks: bool,
) -> Result<Vec<DepOutcome>> {
    let dirs = gui.dirs.clone();
    let title = format!("Installing {} into {id}", verbs.join(", "));
    tasks::run(app, task, title, move |progress| {
        bottles::install_deps(&dirs, &id, &verbs, force, winetricks, progress)
    })
    .await
}

// --- programs ------------------------------------------------------------------

#[tauri::command]
pub async fn add_app(
    gui: State<'_, Gui>,
    bottle: String,
    path: PathBuf,
    name: Option<String>,
) -> Result<AppView> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || apps::add(&dirs, &bottle, &path, name.as_deref())).await
}

#[tauri::command]
pub async fn remove_app(gui: State<'_, Gui>, bottle: String, path: String) -> Result<()> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || apps::remove(&dirs, &bottle, &path)).await
}

#[derive(Clone, Serialize)]
pub struct Started {
    pub log: String,
}

#[derive(Clone, Serialize)]
struct Exited {
    bottle: String,
    label: String,
    code: Option<i32>,
    error: Option<String>,
}

/// Reap a background program and tell the page (`process://exited`) so it can
/// refresh the running state.
fn watch(app: AppHandle, bottle: String, mut spawned: Spawned) -> Started {
    let log = spawned.log.display().to_string();
    std::thread::spawn(move || {
        let (code, error) = match spawned.child.wait() {
            Ok(status) => (status.code(), None),
            Err(e) => (None, Some(e.to_string())),
        };
        tracing::info!(bottle = %bottle, program = %spawned.label, ?code, "program exited");
        let _ = app.emit(
            "process://exited",
            Exited {
                bottle,
                label: spawned.label,
                code,
                error,
            },
        );
    });
    Started { log }
}

#[tauri::command]
pub async fn run_program(
    app: AppHandle,
    gui: State<'_, Gui>,
    bottle: String,
    target: String,
) -> Result<Started> {
    let dirs = gui.dirs.clone();
    let id = bottle.clone();
    let spawned = tasks::blocking(move || apps::spawn_program(&dirs, &id, &target)).await?;
    Ok(watch(app, bottle, spawned))
}

#[tauri::command]
pub async fn run_tool(
    app: AppHandle,
    gui: State<'_, Gui>,
    bottle: String,
    tool: String,
) -> Result<Started> {
    let dirs = gui.dirs.clone();
    let id = bottle.clone();
    let spawned = tasks::blocking(move || {
        let tool = bottles::parse_tool(&tool)?;
        apps::spawn_tool(&dirs, &id, tool)
    })
    .await?;
    Ok(watch(app, bottle, spawned))
}

// --- library & Steam -----------------------------------------------------------

#[tauri::command]
pub async fn library(gui: State<'_, Gui>) -> Result<Library> {
    let dirs = gui.dirs.clone();
    let compat = gui.compat.clone();
    tasks::blocking(move || library::load(&dirs, compat.as_ref())).await
}

#[tauri::command]
pub async fn cover(gui: State<'_, Gui>, appid: u32, fetch: bool) -> Result<Option<String>> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || library::cover(&dirs, appid, fetch)).await
}

#[tauri::command]
pub async fn launch_game(gui: State<'_, Gui>, bottle: String, appid: u32) -> Result<GameLaunched> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || library::launch_game(&dirs, &bottle, appid)).await
}

#[tauri::command]
pub async fn install_steam(
    app: AppHandle,
    gui: State<'_, Gui>,
    task: String,
    bottle: String,
) -> Result<u32> {
    let dirs = gui.dirs.clone();
    let title = format!("Installing Steam into {bottle}");
    tasks::run(app, task, title, move |progress| {
        tracing::info!(bottle = %bottle, "installing Steam from the GUI");
        let report = cross_steam::install::install(&dirs, &bottle, false, progress)?;
        Ok(report.patches_revision)
    })
    .await
}

/// Start the Steam client (with its patches) and return right away. Its
/// output is not captured: `cross_steam::launch::run_client` inherits stdio.
#[tauri::command]
pub async fn start_steam(app: AppHandle, gui: State<'_, Gui>, bottle: String) -> Result<()> {
    let dirs = gui.dirs.clone();
    // Fail fast (no Steam, no engine) before handing off to a thread.
    {
        let (dirs, bottle) = (dirs.clone(), bottle.clone());
        tasks::blocking(move || {
            let (_, runtime) = cross_bottle::Runtime::open(&dirs, &bottle)?;
            if !cross_steam::install::is_installed(runtime.prefix()) {
                return Err(cross_steam::Error::NotInstalled(bottle).into());
            }
            Ok(())
        })
        .await?;
    }
    std::thread::spawn(move || {
        tracing::info!(bottle = %bottle, "starting Steam from the GUI");
        let result = cross_steam::launch::run_client(&dirs, &bottle, &[]);
        let (code, error) = match result {
            Ok(status) => (status.code(), None),
            Err(e) => {
                notify(&app, "Steam", &format!("Steam failed: {e}"));
                (None, Some(e.to_string()))
            }
        };
        let _ = app.emit(
            "process://exited",
            Exited {
                bottle,
                label: "Steam".into(),
                code,
                error,
            },
        );
    });
    Ok(())
}

// --- logs ----------------------------------------------------------------------

#[tauri::command]
pub async fn list_logs(gui: State<'_, Gui>, bottle: Option<String>) -> Result<Vec<LogFile>> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || logs::list(&dirs, bottle.as_deref())).await
}

#[tauri::command]
pub async fn read_log(
    gui: State<'_, Gui>,
    bottle: Option<String>,
    name: String,
    offset: Option<u64>,
) -> Result<LogChunk> {
    let dirs = gui.dirs.clone();
    tasks::blocking(move || logs::read(&dirs, bottle.as_deref(), &name, offset)).await
}

// --- native dialogs ------------------------------------------------------------

/// Native open panel. `kind`: `program` (.exe and friends), `gptk` (.dmg or a
/// mounted folder is picked with `gptk-folder`).
#[tauri::command]
pub async fn pick_file(app: AppHandle, kind: String) -> Result<Option<String>> {
    tasks::blocking(move || {
        let dialog = app.dialog().file();
        let picked = match kind.as_str() {
            "program" => dialog
                .set_title("Choose a Windows program")
                .add_filter("Windows programs", &PROGRAM_EXTENSIONS)
                .blocking_pick_file(),
            "gptk" => dialog
                .set_title("Choose Game_Porting_Toolkit_*.dmg")
                .add_filter("Disk image", &["dmg"])
                .blocking_pick_file(),
            "gptk-folder" => dialog
                .set_title("Choose the mounted Game Porting Toolkit volume")
                .set_directory("/Volumes")
                .blocking_pick_folder(),
            other => return Err(Error::InvalidInput(format!("unknown file kind {other:?}"))),
        };
        picked
            .map(|p| {
                p.into_path()
                    .map(|p| p.display().to_string())
                    .map_err(|e| Error::Internal(e.to_string()))
            })
            .transpose()
    })
    .await
}

/// Native confirmation sheet; `true` when the user pressed `ok`.
#[tauri::command]
pub async fn confirm(
    app: AppHandle,
    title: String,
    message: String,
    ok: Option<String>,
    danger: Option<bool>,
) -> Result<bool> {
    tasks::blocking(move || {
        let kind = if danger.unwrap_or(false) {
            MessageDialogKind::Warning
        } else {
            MessageDialogKind::Info
        };
        Ok(app
            .dialog()
            .message(message)
            .title(title)
            .kind(kind)
            .buttons(MessageDialogButtons::OkCancelCustom(
                ok.unwrap_or_else(|| "OK".into()),
                "Cancel".into(),
            ))
            .blocking_show())
    })
    .await
}
