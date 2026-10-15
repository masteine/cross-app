//! `cross crossover ...`: use an installed CrossOver as an external engine.
//!
//! Not verified against a real CrossOver.app (none was installed while this was
//! written); see `docs/adr/0003-post-rosetta.md`.

use std::process::ExitCode;

use anyhow::{anyhow, Result};
use clap::Subcommand;
use cross_bottle::crossover::{self as import, ImportOptions};
use cross_core::paths::AppDirs;
use cross_engine::catalog::Catalog;
use cross_engine::crossover::{self, CrossOverApp, PrefixArch};
use cross_engine::macho::Arch;
use cross_engine::manager::{EngineManager, InstallOptions};

/// Catalog id of the CrossOver engine.
const ENGINE_ID: &str = "crossover";

#[derive(Subcommand)]
pub enum CrossoverCommand {
    /// Show whether CrossOver is installed and registered as an engine
    Status {
        #[arg(long)]
        json: bool,
    },
    /// List CrossOver bottles
    Bottles {
        #[arg(long)]
        json: bool,
    },
    /// Link a CrossOver bottle into cross-app (nothing is copied or changed)
    Import {
        /// CrossOver bottle name, as shown by `cross crossover bottles`
        bottle: String,
        /// cross-app bottle id (default: derived from the name)
        #[arg(long)]
        id: Option<String>,
        /// Display name (default: the CrossOver bottle name)
        #[arg(long)]
        name: Option<String>,
        /// External engine to run it with
        #[arg(long, default_value = ENGINE_ID)]
        engine: String,
    },
}

pub fn run(command: CrossoverCommand, dirs: &AppDirs) -> Result<ExitCode> {
    match command {
        CrossoverCommand::Status { json } => status(dirs, json),
        CrossoverCommand::Bottles { json } => bottles(json),
        CrossoverCommand::Import {
            bottle,
            id,
            name,
            engine,
        } => import_bottle(dirs, &bottle, id, name, engine),
    }
}

fn arch_name(arch: Option<PrefixArch>) -> &'static str {
    match arch {
        Some(PrefixArch::Win64) => "win64",
        Some(PrefixArch::Win32) => "win32",
        None => "unknown",
    }
}

/// What the Wine loader inside the app says about Rosetta.
fn loader_summary(app: &CrossOverApp) -> String {
    match app.loader_architectures() {
        Some(archs) if archs.contains(&Arch::Arm64) => format!(
            "{} (runs without Rosetta 2)",
            archs.iter().map(|a| a.name()).collect::<Vec<_>>().join("+")
        ),
        Some(archs) => format!(
            "{} only (runs through Rosetta 2)",
            archs.iter().map(|a| a.name()).collect::<Vec<_>>().join("+")
        ),
        None => "unknown".to_owned(),
    }
}

fn status(dirs: &AppDirs, json: bool) -> Result<ExitCode> {
    let candidates = CrossOverApp::candidates();
    let app = CrossOverApp::find(&candidates);
    let registered = EngineManager::new(dirs).get(ENGINE_ID)?;
    let bottle_dirs = crossover::bottle_dirs();
    if json {
        let output = serde_json::json!({
            "app": app.as_ref().map(|a| a.app.display().to_string()),
            "version": app.as_ref().and_then(|a| a.version.clone()),
            "wine_loader": app.as_ref().map(loader_summary),
            "searched": candidates.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "registered": registered.is_some(),
            "bottle_dirs": bottle_dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(ExitCode::SUCCESS);
    }
    match &app {
        Some(app) => {
            println!(
                "CrossOver {} at {}",
                app.version.as_deref().unwrap_or("(unknown version)"),
                app.app.display()
            );
            println!("Wine loader: {}", loader_summary(app));
        }
        None => println!(
            "CrossOver not found (looked in: {})",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
    println!(
        "Engine `{ENGINE_ID}`: {}",
        if registered.is_some() {
            "registered"
        } else {
            "not registered (cross engine install crossover)"
        }
    );
    for dir in &bottle_dirs {
        println!("Bottle folder: {}", dir.display());
    }
    Ok(ExitCode::SUCCESS)
}

fn bottles(json: bool) -> Result<ExitCode> {
    let found = crossover::list_bottles(&crossover::bottle_dirs());
    if json {
        let rows: Vec<_> = found
            .iter()
            .map(|b| {
                serde_json::json!({
                    "name": b.name,
                    "path": b.path.display().to_string(),
                    "arch": arch_name(b.arch),
                    "template": b.template,
                    "crossover_version": b.crossover_version,
                    "suggested_id": import::suggested_id(&b.name),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(ExitCode::SUCCESS);
    }
    if found.is_empty() {
        println!("No CrossOver bottles found.");
        return Ok(ExitCode::SUCCESS);
    }
    let width = found.iter().map(|b| b.name.len()).max().unwrap_or(4).max(4);
    println!(
        "{:width$}  {:7}  {:10}  {:14}  PATH",
        "NAME", "ARCH", "TEMPLATE", "CROSSOVER"
    );
    for b in &found {
        println!(
            "{:width$}  {:7}  {:10}  {:14}  {}",
            b.name,
            arch_name(b.arch),
            b.template.as_deref().unwrap_or("-"),
            b.crossover_version.as_deref().unwrap_or("-"),
            b.path.display()
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn import_bottle(
    dirs: &AppDirs,
    name: &str,
    id: Option<String>,
    display: Option<String>,
    engine: String,
) -> Result<ExitCode> {
    let source = crossover::find_bottle(&crossover::bottle_dirs(), name).ok_or_else(|| {
        anyhow!("no CrossOver bottle named {name:?}; see `cross crossover bottles`")
    })?;
    if source.arch == Some(PrefixArch::Win32) {
        eprintln!(
            "warning: {name:?} is a 32-bit bottle; CodeWeavers says CrossOver 27 no longer runs them"
        );
    }

    let manager = EngineManager::new(dirs);
    if manager.get(&engine)?.is_none() && engine == ENGINE_ID {
        let registered = manager.install(
            &Catalog::load(dirs)?,
            ENGINE_ID,
            InstallOptions::default(),
            &mut |_| {},
        )?;
        println!(
            "Registered CrossOver {} as engine `{ENGINE_ID}`.",
            registered.wine_version
        );
    }

    let bottle = import::import(
        dirs,
        &source,
        &ImportOptions {
            id,
            name: display,
            engine,
        },
    )?;
    println!(
        "Imported {:?} as bottle `{}` (linked, not copied). Run programs with `cross run {} ...`.",
        source.name, bottle.id, bottle.id
    );
    Ok(ExitCode::SUCCESS)
}
