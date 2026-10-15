//! Tauri wiring: state, plugins, menu and the command list.

mod commands;
mod menu;
mod tasks;

use std::sync::Arc;

use cross_core::config::Config;
use cross_core::logging::{self, LogTarget};
use cross_core::paths::AppDirs;

use crate::api::compat::{CompatSource, NoCompatData, ProfileDb};

/// Shared by every command.
pub struct Gui {
    pub dirs: AppDirs,
    /// Ratings for library badges; a placeholder until `cross-compat` (Sprint 7).
    pub compat: Arc<dyn CompatSource>,
}

/// Start the app. Blocks until the last window closes.
pub fn run() {
    let dirs = match AppDirs::discover() {
        Ok(dirs) => dirs,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let config = Config::load(&dirs).unwrap_or_else(|e| {
        eprintln!("warning: {e}; using default settings");
        Config::default()
    });
    // Same files as the CLI (`<data>/logs/`); logging must never stop the app.
    let _log_guard = logging::init(&dirs, LogTarget::Global, &config.log_level)
        .map_err(|e| eprintln!("warning: file logging disabled: {e}"))
        .ok();
    tracing::info!(data_dir = %dirs.root().display(), "starting the GUI");

    let compat: Arc<dyn CompatSource> = match ProfileDb::load(&dirs) {
        Ok(db) => Arc::new(db),
        Err(e) => {
            tracing::warn!(error = %e, "compatibility profiles unavailable; badges show unknown");
            Arc::new(NoCompatData)
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(Gui { dirs, compat })
        .manage(tasks::TaskBoard::default())
        .menu(menu::build)
        .on_menu_event(menu::on_event)
        .invoke_handler(tauri::generate_handler![
            commands::system_status,
            commands::install_rosetta,
            commands::list_engines,
            commands::install_engine,
            commands::remove_engine,
            commands::set_default_engine,
            commands::import_gptk,
            commands::list_bottles,
            commands::bottle_detail,
            commands::create_bottle,
            commands::delete_bottle,
            commands::set_windows_version,
            commands::set_bottle_engine,
            commands::bottle_processes,
            commands::stop_bottle,
            commands::install_deps,
            commands::add_app,
            commands::remove_app,
            commands::run_program,
            commands::run_tool,
            commands::library,
            commands::cover,
            commands::launch_game,
            commands::install_steam,
            commands::start_steam,
            commands::list_logs,
            commands::read_log,
            commands::pick_file,
            commands::confirm,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the cross-app window");
}
