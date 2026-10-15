//! `cross bottle ...` and `cross run`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, bail, Result};
use clap::{Subcommand, ValueEnum};
use cross_bottle::launch::{self, Flags, LaunchRequest, Tool};
use cross_bottle::manager::{BottleManager, CreateOptions};
use cross_bottle::presets::parse_resolution;
use cross_bottle::processes::{running_wineservers, StopMode};
use cross_bottle::registry::RegType;
use cross_bottle::Runtime;
use cross_core::config::Config;
use cross_core::model::{LaunchProfile, WindowsVersion};
use cross_core::paths::AppDirs;

use crate::prompt::confirm;

#[derive(Clone, Copy, ValueEnum)]
pub enum WindowsArg {
    #[value(name = "7")]
    Win7,
    #[value(name = "8")]
    Win8,
    #[value(name = "10")]
    Win10,
    #[value(name = "11")]
    Win11,
}

impl From<WindowsArg> for WindowsVersion {
    fn from(arg: WindowsArg) -> Self {
        match arg {
            WindowsArg::Win7 => WindowsVersion::Win7,
            WindowsArg::Win8 => WindowsVersion::Win8,
            WindowsArg::Win10 => WindowsVersion::Win10,
            WindowsArg::Win11 => WindowsVersion::Win11,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ToolArg {
    Winecfg,
    Regedit,
    Cmd,
    Taskmgr,
}

#[derive(Subcommand)]
pub enum BottleCommand {
    /// Create a bottle (a fresh 64-bit Wine prefix)
    Create {
        /// Short id used on disk and on the command line, e.g. `steam`
        id: String,
        /// Display name (default: the id)
        #[arg(long)]
        name: Option<String>,
        /// Engine to use (default: the configured or only installed engine)
        #[arg(long)]
        engine: Option<String>,
        /// Windows version to report to programs
        #[arg(long, value_enum, default_value = "11")]
        windows: WindowsArg,
    },
    /// List bottles
    List {
        #[arg(long)]
        json: bool,
    },
    /// Delete a bottle and everything in it
    Delete {
        id: String,
        /// Do not ask for confirmation
        #[arg(short, long)]
        yes: bool,
    },
    /// Copy a bottle (instant on APFS)
    Clone {
        source: String,
        target: String,
        /// Display name of the copy
        #[arg(long)]
        name: Option<String>,
    },
    /// Change a bottle's id
    Rename { id: String, new_id: String },
    /// Show the Windows processes running in a bottle
    Ps {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Stop a bottle's processes (ask them to exit; --force kills them)
    Kill {
        id: String,
        #[arg(long)]
        force: bool,
    },
    /// Read or change registry values
    Reg {
        id: String,
        #[command(subcommand)]
        command: RegCommand,
    },
    /// Apply a settings preset: Windows version, DPI, virtual desktop
    Preset {
        id: String,
        #[command(subcommand)]
        command: PresetCommand,
    },
    /// Open a Wine tool for the bottle: winecfg, regedit, cmd, taskmgr
    Tool {
        id: String,
        #[arg(value_enum)]
        tool: ToolArg,
    },
}

#[derive(Subcommand)]
pub enum RegCommand {
    /// Print a value
    Get { key: String, name: String },
    /// Write a value
    Set {
        key: String,
        name: String,
        data: String,
        /// Value type: sz, expand_sz, multi_sz, dword, qword, binary
        #[arg(long = "type", default_value = "sz")]
        kind: String,
    },
    /// Delete a value
    Delete { key: String, name: String },
}

#[derive(Subcommand)]
pub enum PresetCommand {
    /// Windows version reported to programs
    Windows {
        #[arg(value_enum)]
        version: WindowsArg,
    },
    /// Screen DPI: 96 = 100 %, 144 = 150 %, 192 = 200 %
    Dpi { value: u32 },
    /// Fixed-size window for every program, e.g. 1920x1080, or `off`
    VirtualDesktop { size: String },
}

/// Bottle that a command operates on and that should receive its log file.
pub fn log_bottle(command: &BottleCommand) -> Option<&str> {
    match command {
        BottleCommand::Ps { id, .. }
        | BottleCommand::Kill { id, .. }
        | BottleCommand::Reg { id, .. }
        | BottleCommand::Preset { id, .. }
        | BottleCommand::Tool { id, .. } => Some(id),
        _ => None,
    }
}

pub fn run(command: BottleCommand, dirs: &AppDirs, config: &Config) -> Result<ExitCode> {
    let manager = BottleManager::new(dirs);
    match command {
        BottleCommand::Create {
            id,
            name,
            engine,
            windows,
        } => {
            let engine = manager.resolve_engine(engine.as_deref(), config)?;
            eprintln!("Creating bottle {id} with {engine} (this takes about half a minute) ...");
            let bottle = manager.create(&CreateOptions {
                id,
                name,
                engine,
                windows: windows.into(),
            })?;
            println!(
                "Created bottle {} ({}). Run something with: cross run {} <program.exe>",
                bottle.id, bottle.name, bottle.id
            );
        }
        BottleCommand::List { json } => list(dirs, &manager, json)?,
        BottleCommand::Delete { id, yes } => {
            manager.get(&id)?;
            if !confirm(
                &format!("Delete bottle {id} and everything installed in it?"),
                yes,
            )? {
                println!("Cancelled.");
                return Ok(ExitCode::from(1));
            }
            manager.delete(&id)?;
            println!("Deleted bottle {id}.");
        }
        BottleCommand::Clone {
            source,
            target,
            name,
        } => {
            let bottle = manager.clone_bottle(&source, &target, name.as_deref())?;
            println!("Cloned {source} to {} ({}).", bottle.id, bottle.name);
        }
        BottleCommand::Rename { id, new_id } => {
            manager.rename(&id, &new_id)?;
            println!("Renamed {id} to {new_id}.");
        }
        BottleCommand::Ps { id, json } => ps(dirs, &id, json)?,
        BottleCommand::Kill { id, force } => {
            let (_, runtime) = Runtime::open(dirs, &id)?;
            let mode = if force {
                StopMode::Force
            } else {
                StopMode::Graceful
            };
            let outcome = runtime.stop(mode)?;
            if !outcome.was_running {
                println!("Bottle {id} is not running.");
            } else if outcome.remaining == 0 {
                println!("Stopped bottle {id}.");
            } else {
                bail!("bottle {id} is still running; try `cross bottle kill {id} --force`");
            }
        }
        BottleCommand::Reg { id, command } => reg(dirs, &id, command)?,
        BottleCommand::Preset { id, command } => preset(dirs, &id, command)?,
        BottleCommand::Tool { id, tool } => {
            let (_, runtime) = Runtime::open(dirs, &id)?;
            let tool = match tool {
                ToolArg::Winecfg => Tool::Winecfg,
                ToolArg::Regedit => Tool::Regedit,
                ToolArg::Cmd => Tool::Cmd,
                ToolArg::Taskmgr => Tool::Taskmgr,
            };
            return Ok(exit_code(launch::run_tool(&runtime, tool)?));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn list(dirs: &AppDirs, manager: &BottleManager<'_>, json: bool) -> Result<()> {
    let bottles = manager.list()?;
    let rows: Vec<_> = bottles
        .iter()
        .map(|b| {
            let running = running_wineservers(&dirs.bottle_prefix(&b.id))
                .map(|p| !p.is_empty())
                .unwrap_or(false);
            (b, running)
        })
        .collect();

    if json {
        let value: Vec<_> = rows
            .iter()
            .map(|(b, running)| {
                serde_json::json!({
                    "id": b.id,
                    "name": b.name,
                    "engine": b.engine,
                    "windows": b.windows_version.wine_name(),
                    "running": running,
                    "created_unix": b.created_unix,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    if rows.is_empty() {
        println!("No bottles yet. Create one: cross bottle create <id>");
        return Ok(());
    }
    let width = rows
        .iter()
        .map(|(b, _)| b.id.len())
        .max()
        .unwrap_or(2)
        .max(2);
    println!("{:width$}  {:8}  {:7}  ENGINE", "ID", "STATE", "WINDOWS");
    for (b, running) in rows {
        println!(
            "{:width$}  {:8}  {:7}  {}",
            b.id,
            if running { "running" } else { "stopped" },
            b.windows_version.wine_name(),
            b.engine.as_deref().unwrap_or("-"),
        );
    }
    Ok(())
}

fn ps(dirs: &AppDirs, id: &str, json: bool) -> Result<()> {
    let (_, runtime) = Runtime::open(dirs, id)?;
    let processes = runtime.processes()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&processes)?);
    } else if processes.is_empty() {
        println!("Bottle {id} is not running.");
    } else {
        println!("{:>6}  NAME", "PID");
        for p in processes {
            println!("{:>6}  {}", p.pid, p.name);
        }
    }
    Ok(())
}

fn reg(dirs: &AppDirs, id: &str, command: RegCommand) -> Result<()> {
    let (_, runtime) = Runtime::open(dirs, id)?;
    match command {
        RegCommand::Get { key, name } => match runtime.reg_get(&key, &name)? {
            Some(v) => println!("{}\t{}\t{}", v.name, v.kind, v.data),
            None => bail!("{key}\\{name} does not exist"),
        },
        RegCommand::Set {
            key,
            name,
            data,
            kind,
        } => {
            let kind = RegType::parse(&kind).ok_or_else(|| {
                anyhow!("unknown registry type {kind:?}; use sz, expand_sz, multi_sz, dword, qword or binary")
            })?;
            runtime.reg_set(&key, &name, kind, &data)?;
            println!("Set {key}\\{name}.");
        }
        RegCommand::Delete { key, name } => {
            runtime.reg_delete(&key, &name)?;
            println!("Deleted {key}\\{name}.");
        }
    }
    Ok(())
}

fn preset(dirs: &AppDirs, id: &str, command: PresetCommand) -> Result<()> {
    let (_, runtime) = Runtime::open(dirs, id)?;
    match command {
        PresetCommand::Windows { version } => {
            let version = WindowsVersion::from(version);
            runtime.set_windows_version(version)?;
            // Keep bottle.toml in step with the registry.
            let store = cross_core::config::BottleStore::new(dirs);
            let mut bottle = store.load(id)?;
            bottle.windows_version = version;
            store.save(&bottle)?;
            println!("Windows version set to {}.", version.wine_name());
        }
        PresetCommand::Dpi { value } => {
            runtime.set_dpi(value)?;
            println!("DPI set to {value}.");
        }
        PresetCommand::VirtualDesktop { size } => {
            if size.eq_ignore_ascii_case("off") {
                runtime.disable_virtual_desktop()?;
                println!("Virtual desktop disabled.");
            } else {
                let (w, h) = parse_resolution(&size)
                    .ok_or_else(|| anyhow!("size must look like 1920x1080 or `off`"))?;
                runtime.enable_virtual_desktop(w, h)?;
                println!("Virtual desktop enabled: {w}x{h}.");
            }
        }
    }
    Ok(())
}

/// Arguments of `cross run`.
pub struct RunArgs {
    pub bottle: String,
    pub target: String,
    pub workdir: Option<PathBuf>,
    pub env: Vec<String>,
    pub dll_override: Vec<String>,
    pub flags: Flags,
    pub options: launch::RunOptions,
    pub args: Vec<String>,
}

pub fn run_program(args: RunArgs, dirs: &AppDirs) -> Result<ExitCode> {
    let (_, runtime) = Runtime::open(dirs, &args.bottle)?;
    // Given a folder, Wine opens it in its file manager (explorer.exe) and
    // `cross run` blocks until that window is closed.
    if std::path::Path::new(args.target.trim()).is_dir() {
        bail!(
            "{} is a folder; pass the program inside it (.exe, .msi, .lnk or .bat)",
            args.target
        );
    }

    let mut profile = LaunchProfile::default();
    for item in &args.env {
        let (k, v) = item
            .split_once('=')
            .filter(|(k, _)| !k.is_empty())
            .ok_or_else(|| anyhow!("--env expects KEY=VALUE, got {item:?}"))?;
        profile.env.insert(k.to_owned(), v.to_owned());
    }
    for item in &args.dll_override {
        let (dll, mode) = item
            .split_once('=')
            .filter(|(dll, _)| !dll.is_empty())
            .ok_or_else(|| {
                anyhow!("--dll-override expects DLL=MODE (e.g. d3d11=n,b), got {item:?}")
            })?;
        profile
            .dll_overrides
            .insert(dll.to_owned(), mode.to_owned());
    }

    let outcome = launch::run_with(
        &runtime,
        &LaunchRequest {
            target: args.target,
            args: args.args,
            working_dir: args.workdir,
            profile,
            flags: args.flags,
        },
        &args.options,
    )?;
    if let (false, Some(log)) = (outcome.status.success(), &outcome.log) {
        eprintln!(
            "Program exited with {}; Wine output: {}",
            outcome.status,
            log.display()
        );
    }
    Ok(exit_code(outcome.status))
}

fn exit_code(status: std::process::ExitStatus) -> ExitCode {
    match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(code) => ExitCode::from(u8::try_from(code & 0xff).unwrap_or(1).max(1)),
        None => ExitCode::from(1), // killed by a signal
    }
}
