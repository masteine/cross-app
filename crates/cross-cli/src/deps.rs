//! `cross deps ...`: install Windows dependencies (vcrun2022, dxvk, ...) into bottles.

use std::process::ExitCode;

use anyhow::Result;
use clap::Subcommand;
use cross_core::config::BottleStore;
use cross_core::model::DepSource;
use cross_core::paths::AppDirs;
use cross_deps::install::{InstallOptions, Installer, Status};

use crate::progress::ProgressRenderer;

#[derive(Subcommand)]
pub enum DepsCommand {
    /// List the verbs we can install (and which are installed in a bottle)
    List {
        /// Show install state for this bottle
        #[arg(long)]
        bottle: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Install verbs into a bottle, e.g. `cross deps install demo vcrun2022 dxvk`
    Install {
        bottle: String,
        #[arg(required = true)]
        verbs: Vec<String>,
        /// Reinstall verbs the bottle already has
        #[arg(long)]
        force: bool,
        /// Never fall back to the system winetricks for verbs outside our catalog
        #[arg(long)]
        no_winetricks: bool,
    },
}

/// Bottle that should receive this command's log output.
pub fn log_bottle(command: &DepsCommand) -> Option<&str> {
    match command {
        DepsCommand::Install { bottle, .. } => Some(bottle),
        DepsCommand::List { .. } => None,
    }
}

pub fn run(command: DepsCommand, dirs: &AppDirs) -> Result<ExitCode> {
    let installer = Installer::new(dirs)?;
    match command {
        DepsCommand::List { bottle, json } => list(dirs, &installer, bottle.as_deref(), json)?,
        DepsCommand::Install {
            bottle,
            verbs,
            force,
            no_winetricks,
        } => {
            let options = InstallOptions {
                force,
                allow_winetricks: !no_winetricks,
            };
            if options.allow_winetricks {
                warn_about_winetricks(&installer, &verbs);
            }
            let mut renderer = ProgressRenderer::new();
            let result = installer.install(&bottle, &verbs, options, &mut |p| renderer.update(p));
            renderer.finish();
            for outcome in result? {
                match outcome.status {
                    Status::Installed(DepSource::Catalog) => {
                        println!("{}: installed", outcome.verb)
                    }
                    Status::Installed(DepSource::Winetricks) => {
                        println!("{}: installed (via winetricks)", outcome.verb);
                    }
                    Status::AlreadyInstalled => {
                        println!(
                            "{}: already installed (use --force to reinstall)",
                            outcome.verb
                        );
                    }
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// winetricks is a third-party script that downloads and runs installers: say so
/// before using it for a verb of ours that does not exist.
fn warn_about_winetricks(installer: &Installer<'_>, verbs: &[String]) {
    let catalog = installer.catalog();
    let outside: Vec<&str> = catalog
        .resolve_order(verbs)
        .into_iter()
        .filter(|v| catalog.get(v).is_none())
        .collect();
    if let (false, Some(path)) = (outside.is_empty(), installer.winetricks_path()) {
        eprintln!(
            "note: {} not in our catalog; using {} (a third-party script that downloads and runs installers)",
            outside.join(", "),
            path.display()
        );
    }
}

fn list(dirs: &AppDirs, installer: &Installer<'_>, bottle: Option<&str>, json: bool) -> Result<()> {
    let installed = match bottle {
        Some(id) => Some(BottleStore::new(dirs).load(id)?.deps),
        None => None,
    };
    let is_installed = |verb: &str| installed.as_ref().map(|d| d.contains_key(verb));

    if json {
        let rows: Vec<_> = installer
            .catalog()
            .verbs
            .iter()
            .map(|(name, verb)| {
                serde_json::json!({
                    "verb": name,
                    "title": verb.title,
                    "installed": is_installed(name),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    let width = installer
        .catalog()
        .verbs
        .keys()
        .map(String::len)
        .max()
        .unwrap_or(4)
        .max(4);
    println!("{:width$}  {:9}  TITLE", "VERB", "STATE");
    for (name, verb) in &installer.catalog().verbs {
        let state = match is_installed(name) {
            Some(true) => "installed",
            Some(false) => "-",
            None => "",
        };
        println!("{name:width$}  {state:9}  {}", verb.title);
    }
    if let Some(deps) = &installed {
        for (name, record) in deps
            .iter()
            .filter(|(n, _)| installer.catalog().get(n).is_none())
        {
            println!("{name:width$}  {:9}  ({:?})", "installed", record.source);
        }
    }
    match installer.winetricks_path() {
        Some(path) => println!("\nOther verbs: system winetricks at {}", path.display()),
        None => println!(
            "\nOther verbs need winetricks (brew install winetricks); it is not installed."
        ),
    }
    Ok(())
}
