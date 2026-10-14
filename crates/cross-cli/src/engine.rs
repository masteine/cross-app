//! `cross engine ...`: list, install and remove engines; import GPTK; install Rosetta.

use crate::progress::ProgressRenderer;
use crate::prompt::confirm;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::Subcommand;
use cross_core::config::Config;
use cross_core::model::Translation;
use cross_core::paths::AppDirs;
use cross_core::system::{self, GptkInfo};
use cross_engine::catalog::Catalog;
use cross_engine::manager::{EngineManager, InstallOptions};
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
    /// Download, verify and install an engine from the catalog
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
    /// Install Rosetta 2 (asks for confirmation first)
    InstallRosetta {
        /// Do not ask for confirmation
        #[arg(short, long)]
        yes: bool,
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
        EngineCommand::InstallRosetta { yes } => install_rosetta(yes),
    }
}

fn translation_name(t: Translation) -> &'static str {
    match t {
        Translation::Rosetta => "rosetta",
        Translation::Native => "native",
        Translation::External => "external",
    }
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
                "translation": translation_name(e.translation),
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
            "translation": translation_name(local.translation),
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
        if entry.requires_rosetta() && !system::rosetta_installed() {
            eprintln!(
                "warning: {id} needs Rosetta 2, which is not installed; \
                 run `cross engine install-rosetta`"
            );
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
