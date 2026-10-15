//! The `cross` command-line interface.

mod bottle;
mod compat;
mod crash;
mod deps;
mod diag;
mod engine;
mod logs;
mod progress;
mod prompt;
mod steam;

use std::process::ExitCode;
use std::time::Instant;

use anyhow::Result;
use clap::{ArgAction, Parser, Subcommand};
use cross_core::config::Config;
use cross_core::doctor::{self, Report, Status};
use cross_core::logging::{self, LogOptions, LogTarget};
use cross_core::paths::AppDirs;
use cross_core::system::SystemInfo;

#[derive(Parser)]
#[command(name = "cross", version, about = "Run Windows games and apps on Apple Silicon Macs", long_about = None)]
struct Cli {
    /// More output on the terminal: -v info, -vv debug, -vvv trace (log files always keep debug)
    #[arg(short, long, action = ArgAction::Count, global = true)]
    verbose: u8,
    /// Only errors on the terminal
    #[arg(short, long, global = true, conflicts_with = "verbose")]
    quiet: bool,
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
        /// Wine debug channels for this run, e.g. +err,+module (WINEDEBUG; default -all)
        #[arg(long, value_name = "CHANNELS")]
        wine_debug: Option<String>,
        /// Keep Wine's stderr on the terminal instead of the bottle's run log
        #[arg(long)]
        no_log: bool,
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
    /// Show log files: cross-app's own, Wine run output, Steam output
    Logs(logs::LogsArgs),
    /// Collect logs, settings and system info into an archive for bug reports
    Diag(diag::DiagArgs),
    /// Game compatibility profiles: search, show, apply, sync
    Compat {
        #[command(subcommand)]
        command: compat::CompatCommand,
    },
    /// Local crash reports (never uploaded): list, show, clear
    Crash {
        #[command(subcommand)]
        command: crash::CrashCommand,
    },
}

impl Command {
    /// Short name for log lines.
    fn name(&self) -> &'static str {
        match self {
            Command::Doctor { .. } => "doctor",
            Command::Engine { .. } => "engine",
            Command::Bottle { .. } => "bottle",
            Command::Run { .. } => "run",
            Command::Deps { .. } => "deps",
            Command::Steam { .. } => "steam",
            Command::Logs(_) => "logs",
            Command::Diag(_) => "diag",
            Command::Compat { .. } => "compat",
            Command::Crash { .. } => "crash",
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {}", cross_core::chain_message(err.as_ref()));
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
        Command::Compat { command } => compat::log_bottle(command),
        Command::Logs(args) => args.bottle.as_deref(),
        Command::Doctor { .. }
        | Command::Engine { .. }
        | Command::Diag(_)
        | Command::Crash { .. } => None,
    }
}

/// Log target of the per-command start/finish lines: file only, the terminal
/// already shows the command's own output and errors.
const LIFECYCLE: &str = "cross::lifecycle";

/// Terminal filter for `-q` / `-v` / `-vv` / `-vvv`.
fn stderr_filter(verbose: u8, quiet: bool) -> String {
    let level = match (quiet, verbose) {
        (true, _) => "error",
        (false, 0) => "warn",
        (false, 1) => "info",
        (false, 2) => "debug",
        (false, _) => return format!("trace,{LIFECYCLE}=off"),
    };
    format!("{level},{},{LIFECYCLE}=off", logging::QUIET_DEPENDENCIES)
}

fn run(cli: Cli) -> Result<ExitCode> {
    let dirs = AppDirs::discover()?;
    let config = Config::load(&dirs)?;

    let version = env!("CARGO_PKG_VERSION");
    // `cross logs` reads the log files; it must not add its own lines to them.
    if let Command::Logs(args) = &cli.command {
        crash::install_hook(&dirs, None, version);
        return logs::run(args, &dirs);
    }

    // Logging must never prevent a command from running; keep the guard alive until exit.
    let target = match log_bottle(&cli.command) {
        Some(id) if dirs.bottle_file(id).is_file() => LogTarget::Bottle(id),
        _ => LogTarget::Global,
    };
    let file_filter = format!("{},{}", config.log_level, logging::QUIET_DEPENDENCIES);
    let stderr_filter = stderr_filter(cli.verbose, cli.quiet);
    let _log_guard = logging::init_with(
        &dirs,
        target,
        &LogOptions {
            file_filter: &file_filter,
            stderr_filter: &stderr_filter,
        },
    )
    .map_err(|err| eprintln!("warning: file logging disabled: {err}"))
    .ok();
    // Panics write a local crash report including the tail of this command's log.
    crash::install_hook(&dirs, _log_guard.as_ref().map(|g| g.dir.clone()), version);

    let name = cli.command.name();
    let span = tracing::info_span!("cmd", name);
    let _entered = span.enter();
    let started = Instant::now();
    tracing::info!(
        target: LIFECYCLE,
        version,
        args = %cross_bottle::exec::redact(&format!("{:?}", std::env::args().skip(1).collect::<Vec<_>>())),
        data_dir = %dirs.root().display(),
        "command started"
    );
    tracing::debug!(
        target: LIFECYCLE,
        rust_log = ?std::env::var("RUST_LOG").ok(),
        winedebug = ?std::env::var("WINEDEBUG").ok(),
        cross_app_home = ?std::env::var("CROSS_APP_HOME").ok(),
        "environment"
    );

    let result = dispatch(cli.command, &dirs, &config);
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match &result {
        Ok(code) => tracing::info!(target: LIFECYCLE, elapsed_ms, exit = ?code, "command finished"),
        Err(err) => tracing::error!(
            target: LIFECYCLE,
            elapsed_ms,
            error = %cross_core::chain_message(err.as_ref()),
            "command failed"
        ),
    }
    result
}

fn dispatch(command: Command, dirs: &AppDirs, config: &Config) -> Result<ExitCode> {
    match command {
        Command::Doctor { json } => doctor_command(dirs, config, json),
        Command::Engine { command } => engine::run(command, dirs, config),
        Command::Bottle { command } => bottle::run(command, dirs, config),
        Command::Deps { command } => deps::run(command, dirs),
        Command::Steam { command } => steam::run(command, dirs, config),
        Command::Logs(args) => logs::run(&args, dirs),
        Command::Diag(args) => diag::run(&args, dirs, config),
        Command::Compat { command } => compat::run(command, dirs),
        Command::Crash { command } => crash::run(command, dirs),
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
            wine_debug,
            no_log,
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
                options: cross_bottle::launch::RunOptions {
                    no_capture: no_log,
                    wine_debug,
                },
                args,
            },
            dirs,
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

    #[test]
    fn verbosity_flags_are_global() {
        let cli = Cli::try_parse_from(["cross", "bottle", "list", "-vv"]).unwrap();
        assert_eq!(cli.verbose, 2);
        let cli = Cli::try_parse_from(["cross", "-q", "doctor"]).unwrap();
        assert!(cli.quiet);
        assert!(Cli::try_parse_from(["cross", "-q", "-v", "doctor"]).is_err());
    }

    #[test]
    fn terminal_filter_follows_verbosity() {
        assert!(stderr_filter(0, false).starts_with("warn,"));
        assert!(stderr_filter(1, false).starts_with("info,"));
        assert!(stderr_filter(2, false).starts_with("debug,"));
        assert!(stderr_filter(3, false).starts_with("trace,"));
        assert!(stderr_filter(1, false).contains("cross::lifecycle=off"));
        assert!(stderr_filter(0, true).starts_with("error,"));
    }

    #[test]
    fn run_accepts_logging_options() {
        let cli = Cli::try_parse_from([
            "cross",
            "run",
            "demo",
            "app.exe",
            "--wine-debug",
            "+err",
            "--no-log",
            "--",
            "-x",
        ])
        .unwrap();
        let Command::Run {
            wine_debug,
            no_log,
            args,
            ..
        } = cli.command
        else {
            panic!("expected run");
        };
        assert_eq!(wine_debug.as_deref(), Some("+err"));
        assert!(no_log);
        assert_eq!(args, ["-x"]);
    }
}
