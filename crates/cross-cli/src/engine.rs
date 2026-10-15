//! `cross engine ...`: list, install and remove engines; import GPTK; install Rosetta.

use crate::progress::ProgressRenderer;
use crate::prompt::confirm;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Subcommand;
use cross_core::config::Config;
use cross_core::paths::AppDirs;
use cross_core::system::{self, GptkInfo};
use cross_engine::catalog::Catalog;
use cross_engine::manager::{EngineManager, InstallOptions};
use cross_engine::policy::{self, Host};
use cross_engine::{gptk, rosetta};

#[derive(Subcommand)]
pub enum EngineCommand {
    /// List engines from the catalog and whether they are installed
    List {
        /// Only show installed engines
        #[arg(long)]
        installed: bool,
        /// Print as JSON
        #[arg(long)]
        json: bool,
    },
    /// Install an engine from the catalog (downloaded, or found on the system for external ones like CrossOver)
    Install {
        /// Engine id, as shown by `cross engine list`
        id: String,
        /// Also install the DXVK + MoltenVK graphics backend
        #[arg(long)]
        dxvk: bool,
    },
    /// Delete an installed engine
    Remove { id: String },
    /// Import D3DMetal from Apple's Game_Porting_Toolkit_*.dmg (or a mounted copy)
    ImportGptk {
        /// Path to the .dmg, or to the folder where it is mounted
        path: PathBuf,
        /// Toolkit version to record (default: taken from the file name)
        #[arg(long)]
        version: Option<String>,
    },
    /// Plug the imported D3DMetal into an engine (an APFS-cloned overlay), or remove it
    D3dmetal {
        #[command(subcommand)]
        command: D3dMetalCommand,
    },
    /// Install Rosetta 2 (asks for confirmation first)
    InstallRosetta {
        /// Do not ask for confirmation
        #[arg(short, long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum D3dMetalCommand {
    /// Build the D3DMetal overlay of an engine from the imported GPTK
    Enable {
        id: String,
        /// Build it even though the engine is not known to run D3DMetal
        /// (upstream Wine crashes in D3DMetal code)
        #[arg(long)]
        force: bool,
    },
    /// Delete the engine's D3DMetal overlay
    Disable { id: String },
    /// Show what the engine supports and whether D3DMetal is set up
    Status {
        id: String,
        #[arg(long)]
        json: bool,
    },
}

pub fn run(command: EngineCommand, dirs: &AppDirs, config: &Config) -> Result<ExitCode> {
    match command {
        EngineCommand::List { installed, json } => list(dirs, config, installed, json),
        EngineCommand::Install { id, dxvk } => install(dirs, &id, dxvk),
        EngineCommand::Remove { id } => {
            EngineManager::new(dirs).remove(&id)?;
            println!("Removed {id}.");
            Ok(ExitCode::SUCCESS)
        }
        EngineCommand::ImportGptk { path, version } => {
            import_gptk(dirs, config, &path, version.as_deref())
        }
        EngineCommand::D3dmetal { command } => d3dmetal(dirs, config, command),
        EngineCommand::InstallRosetta { yes } => install_rosetta(yes),
    }
}

fn d3dmetal(dirs: &AppDirs, config: &Config, command: D3dMetalCommand) -> Result<ExitCode> {
    let manager = EngineManager::new(dirs);
    let gptk_dir = config.gptk_dir(dirs);
    match command {
        D3dMetalCommand::Enable { id, force } => {
            if force {
                eprintln!(
                    "warning: building the D3DMetal overlay of {id} with --force; on upstream Wine \
                     builds programs that use D3DMetal crash (https://github.com/frankea/Whisky/issues/163)"
                );
            }
            let overlay = manager.enable_d3dmetal(&id, &gptk_dir, force)?;
            println!(
                "D3DMetal overlay of {id} ready (GPTK {}) at {}.",
                overlay
                    .record
                    .gptk_version
                    .as_deref()
                    .unwrap_or("unknown version"),
                overlay.dir.display()
            );
            println!("Bottles and games with the d3dmetal backend on {id} now use it.");
        }
        D3dMetalCommand::Disable { id } => {
            if manager.disable_d3dmetal(&id)? {
                println!("Removed the D3DMetal overlay of {id}.");
            } else {
                println!("{id} has no D3DMetal overlay.");
            }
        }
        D3dMetalCommand::Status { id, json } => {
            let engine = manager
                .get(&id)?
                .ok_or_else(|| anyhow::anyhow!("engine {id:?} is not installed"))?;
            let features = cross_engine::features::probe(&manager.wine_path(&engine));
            let overlay = manager.d3dmetal_overlay(&engine)?;
            let payload = gptk::inspect(&gptk_dir);
            if json {
                let value = serde_json::json!({
                    "engine": id,
                    "gptk_capable": engine.gptk_capable,
                    "features": features,
                    "gptk": payload.as_ref().map(|p| serde_json::json!({
                        "dir": p.dir, "version": p.version, "missing": p.missing,
                    })),
                    "overlay": overlay.as_ref().map(|o| serde_json::json!({
                        "dir": o.dir,
                        "gptk_version": o.record.gptk_version,
                        "forced": o.record.forced,
                        "stale": o.is_stale(payload.as_ref()),
                    })),
                });
                println!("{}", serde_json::to_string_pretty(&value)?);
                return Ok(ExitCode::SUCCESS);
            }
            let yes = |b: bool| if b { "yes" } else { "no" };
            println!(
                "Engine {id}: known to run D3DMetal: {}",
                yes(engine.gptk_capable)
            );
            if features.inspected {
                println!(
                    "Detected: esync {}, msync {}, CPU count limit (WINENCPU) {}, Retina mode {}",
                    yes(features.esync),
                    yes(features.msync),
                    yes(features.cpu_count_override),
                    yes(features.retina_mode)
                );
            } else {
                println!("Detected: could not inspect the engine's unix libraries");
            }
            match &payload {
                Some(p) if p.is_complete() => println!(
                    "GPTK import: {} at {}",
                    p.version.as_deref().unwrap_or("unknown version"),
                    p.dir.display()
                ),
                Some(p) => println!(
                    "GPTK import: incomplete (missing {}); re-import it",
                    p.missing.join(", ")
                ),
                None => println!("GPTK import: none (cross engine import-gptk <dmg>)"),
            }
            match &overlay {
                Some(o) => println!(
                    "D3DMetal overlay: {}{}{}",
                    o.dir.display(),
                    if o.record.forced {
                        " (built with --force)"
                    } else {
                        ""
                    },
                    if o.is_stale(payload.as_ref()) {
                        " (older than the GPTK import; run enable again)"
                    } else {
                        ""
                    }
                ),
                None => println!("D3DMetal overlay: none (cross engine d3dmetal enable {id})"),
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn list(dirs: &AppDirs, config: &Config, only_installed: bool, json: bool) -> Result<ExitCode> {
    let catalog = Catalog::load(dirs)?;
    let installed = EngineManager::new(dirs).list()?;
    let gptk_info = GptkInfo::probe(&config.gptk_dir(dirs));
    let is_installed = |id: &str| installed.iter().find(|e| e.id == id);

    let mut rows: Vec<_> = catalog
        .engines
        .iter()
        .filter(|e| !only_installed || is_installed(&e.id).is_some())
        .map(|e| {
            let local = is_installed(&e.id);
            serde_json::json!({
                "id": e.id,
                "name": e.name,
                "wine_version": e.wine_version,
                "translation": e.translation.name(),
                "installed": local.is_some(),
                "dxvk_installed": local.is_some_and(|l| l.has_dxvk()),
                "in_catalog": true,
            })
        })
        .collect();
    // Engines installed from a catalog that no longer lists them.
    for local in installed.iter().filter(|l| catalog.engine(&l.id).is_none()) {
        rows.push(serde_json::json!({
            "id": local.id,
            "name": local.name,
            "wine_version": local.wine_version,
            "translation": local.translation.name(),
            "installed": true,
            "dxvk_installed": local.has_dxvk(),
            "in_catalog": false,
        }));
    }

    if json {
        let output = serde_json::json!({ "engines": rows, "gptk": gptk_info });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(ExitCode::SUCCESS);
    }

    let text = |row: &serde_json::Value, key: &str| row[key].as_str().unwrap_or("").to_owned();
    let id_width = rows
        .iter()
        .map(|r| text(r, "id").len())
        .max()
        .unwrap_or(2)
        .max(2);
    println!(
        "{:id_width$}  {:6}  {:9}  STATUS",
        "ID", "WINE", "TRANSLATE"
    );
    for row in &rows {
        let status = match (row["installed"].as_bool(), row["dxvk_installed"].as_bool()) {
            (Some(true), Some(true)) => "installed (+dxvk)",
            (Some(true), _) => "installed",
            _ => "available",
        };
        println!(
            "{:id_width$}  {:6}  {:9}  {status}",
            text(row, "id"),
            text(row, "wine_version"),
            text(row, "translation"),
        );
    }
    println!();
    match gptk_info {
        Some(info) => println!(
            "Game Porting Toolkit: {} ({})",
            info.version.as_deref().unwrap_or("unknown version"),
            info.dir.display()
        ),
        None => println!("Game Porting Toolkit: not imported (cross engine import-gptk <dmg>)"),
    }
    Ok(ExitCode::SUCCESS)
}

fn install(dirs: &AppDirs, id: &str, dxvk: bool) -> Result<ExitCode> {
    let catalog = Catalog::load(dirs)?;
    if let Some(entry) = catalog.engine(id) {
        for warning in policy::install_warnings(entry, &Host::detect()) {
            eprintln!("warning: {warning}");
        }
    }

    let manager = EngineManager::new(dirs);
    let mut renderer = ProgressRenderer::new();
    // `--dxvk` on an engine that is already installed adds the missing components.
    let result = match manager.get(id)? {
        Some(existing) if dxvk && !existing.has_dxvk() => {
            manager.add_dxvk(&catalog, id, &mut |p| renderer.update(p))
        }
        _ => manager.install(&catalog, id, InstallOptions { with_dxvk: dxvk }, &mut |p| {
            renderer.update(p)
        }),
    };
    renderer.finish();
    let installed = result?;

    if !installed.translation.is_downloaded() {
        println!(
            "Registered {} {} ({}); nothing was downloaded.",
            installed.id,
            installed.wine_version,
            manager.wine_path(&installed).display()
        );
        return Ok(ExitCode::SUCCESS);
    }
    println!(
        "Installed {} (Wine {}){}.",
        installed.id,
        installed.wine_version,
        if installed.has_dxvk() {
            " with DXVK + MoltenVK"
        } else {
            ""
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn import_gptk(
    dirs: &AppDirs,
    config: &Config,
    path: &std::path::Path,
    version: Option<&str>,
) -> Result<ExitCode> {
    let dest = config.gptk_dir(dirs);
    eprintln!("Importing D3DMetal from {} ...", path.display());
    let imported = gptk::import(path, &dest, version)?;
    println!(
        "Imported Game Porting Toolkit {} into {}.",
        imported.version.as_deref().unwrap_or("(unknown version)"),
        imported.dir.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn install_rosetta(yes: bool) -> Result<ExitCode> {
    if system::rosetta_installed() {
        println!("Rosetta 2 is already installed.");
        return Ok(ExitCode::SUCCESS);
    }
    let question = format!("This will run `{}`. Continue?", rosetta::command_line());
    if !confirm(&question, yes)? {
        println!("Cancelled.");
        return Ok(ExitCode::from(1));
    }
    match rosetta::install()? {
        rosetta::Outcome::AlreadyInstalled => println!("Rosetta 2 is already installed."),
        rosetta::Outcome::Installed => println!("Rosetta 2 installed."),
    }
    Ok(ExitCode::SUCCESS)
}
