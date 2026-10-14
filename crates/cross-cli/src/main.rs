//! The `cross` command-line interface.

mod bottle;
mod deps;
mod engine;
mod progress;
mod prompt;
mod steam;

use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};
use cross_core::config::Config;
use cross_core::doctor::{self, Report, Status};
use cross_core::logging::{self, LogTarget};
use cross_core::paths::AppDirs;
use cross_core::system::SystemInfo;

#[derive(Parser)]
#[command(name = "cross", version, about = "Run Windows games and apps on Apple Silicon Macs", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check this Mac: chip, macOS, Rosetta, Xcode tools, Game Porting Toolkit
    Doctor {
        /// Print the full report as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage Wine engines, the Game Porting Toolkit and Rosetta 2
    Engine {
        #[command(subcommand)]
        command: engine::EngineCommand,
    },
    /// Manage bottles (Wine prefixes): create, delete, clone, rename, processes, registry
    Bottle {
        #[command(subcommand)]
        command: bottle::BottleCommand,
    },
    /// Run a .exe, .msi, .lnk or .bat inside a bottle
    Run {
        /// Bottle id
        bottle: String,
        /// Program: a path on this Mac, a Windows path (C:\...) or a name like `notepad`
        target: String,
        /// Working directory (default: the program's folder)
        #[arg(long)]
        workdir: Option<std::path::PathBuf>,
        /// Extra environment variable, KEY=VALUE (repeatable)
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// DLL override, DLL=MODE such as d3d11=n,b (repeatable)
        #[arg(long = "dll-override", value_name = "DLL=MODE")]
        dll_override: Vec<String>,
        /// Enable esync (WINEESYNC=1)
        #[arg(long)]
        esync: bool,
        /// Enable msync (WINEMSYNC=1)
        #[arg(long)]
        msync: bool,
        /// Show Apple's Metal performance HUD (MTL_HUD_ENABLED=1)
        #[arg(long)]
        metal_hud: bool,
        /// DXVK HUD spec, e.g. fps,frametimes (DXVK_HUD)
        #[arg(long, value_name = "SPEC")]
        dxvk_hud: Option<String>,
        /// Arguments passed to the program (after `--`)
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Install dependencies such as vcrun2022 or dxvk into a bottle
    Deps {
        #[command(subcommand)]
        command: deps::DepsCommand,
    },
    /// Install Steam into a bottle, list its games and launch them
    Steam {
        #[command(subcommand)]
        command: steam::SteamCommand,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Bottle whose log file should receive this command's log output.
fn log_bottle(command: &Command) -> Option<&str> {
    match command {
        Command::Bottle { command } => bottle::log_bottle(command),
        Command::Run { bottle, .. } => Some(bottle),
        Command::Deps { command } => deps::log_bottle(command),
        Command::Steam { command } => steam::log_bottle(command),
        Command::Doctor { .. } | Command::Engine { .. } => None,
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let dirs = AppDirs::discover()?;
    let config = Config::load(&dirs)?;
    // Logging must never prevent a command from running; keep the guard alive until exit.
    let target = match log_bottle(&cli.command) {
        Some(id) if dirs.bottle_file(id).is_file() => LogTarget::Bottle(id),
        _ => LogTarget::Global,
    };
    let _log_guard = logging::init(&dirs, target, &config.log_level)
        .map_err(|err| eprintln!("warning: file logging disabled: {err}"))
        .ok();

    match cli.command {
        Command::Doctor { json } => doctor_command(&dirs, &config, json),
        Command::Engine { command } => engine::run(command, &dirs, &config),
        Command::Bottle { command } => bottle::run(command, &dirs, &config),
        Command::Deps { command } => deps::run(command, &dirs),
        Command::Steam { command } => steam::run(command, &dirs, &config),
        Command::Run {
            bottle,
            target,
            workdir,
            env,
            dll_override,
            esync,
            msync,
            metal_hud,
            dxvk_hud,
            args,
        } => bottle::run_program(
            bottle::RunArgs {
                bottle,
                target,
                workdir,
                env,
                dll_override,
                flags: cross_bottle::launch::Flags {
                    esync,
                    msync,
                    metal_hud,
                    dxvk_hud,
                },
                args,
            },
            &dirs,
        ),
    }
}

fn doctor_command(dirs: &AppDirs, config: &Config, json: bool) -> Result<ExitCode> {
    let info = SystemInfo::detect(&config.gptk_dir(dirs));
    let report = doctor::evaluate(&info);
    tracing::info!(failures = report.has_failures(), "doctor finished");

    if json {
        let output = serde_json::json!({ "system": info, "report": report });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_report(&report);
    }
    Ok(if report.has_failures() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn print_report(report: &Report) {
    let width = report
        .checks
        .iter()
        .map(|c| c.name.len())
        .max()
        .unwrap_or(0);
    for check in &report.checks {
        let mark = match check.status {
            Status::Ok => "ok  ",
            Status::Warn => "warn",
            Status::Fail => "FAIL",
        };
        println!("[{mark}] {:width$}  {}", check.name, check.detail);
    }
    println!();
    if report.has_failures() {
        println!("Some checks failed: cross-app cannot run on this system.");
    } else if report.has_warnings() {
        println!("Ready, with warnings above.");
    } else {
        println!("All good.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }
}
