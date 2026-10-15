//! `cross bottle ...` and `cross run`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Subcommand, ValueEnum};
use cross_bottle::graphics::{self, EngineGraphics, GraphicsPlan, SettingsUpdate};
use cross_bottle::input::GamepadOptions;
use cross_bottle::launch::{self, Flags, LaunchRequest, Tool};
use cross_bottle::manager::{BottleManager, CreateOptions};
use cross_bottle::presets::parse_resolution;
use cross_bottle::processes::{running_wineservers, StopMode};
use cross_bottle::registry::RegType;
use cross_bottle::Runtime;
use cross_core::config::BottleStore;
use cross_core::config::Config;
use cross_core::model::{GraphicsBackend, LaunchProfile, SyncMode, WindowsVersion};
use cross_core::paths::AppDirs;
use cross_steam::launch::{game_profile, update_game_profile, validate_backend, ProfileUpdate};

use crate::prompt::confirm;
use crate::steam::BackendArg;
use cross_telemetry::Outcome as TelemetryOutcome;

/// `on`, `off` or `inherit` (from the bottle, or Wine's default).
#[derive(Clone, Copy, ValueEnum)]
pub enum Toggle {
    On,
    Off,
    Inherit,
}

impl Toggle {
    fn update(self) -> Option<bool> {
        match self {
            Toggle::On => Some(true),
            Toggle::Off => Some(false),
            Toggle::Inherit => None,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum OnOff {
    On,
    Off,
}

impl OnOff {
    fn on(self) -> bool {
        matches!(self, OnOff::On)
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum SyncArg {
    None,
    Esync,
    Msync,
    Inherit,
}

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
    /// Show or change graphics settings of a bottle or one of its Steam games
    Graphics {
        id: String,
        /// Change a Steam game's settings instead of the bottle's
        #[arg(long)]
        appid: Option<u32>,
        /// Direct3D backend
        #[arg(long, value_enum)]
        backend: Option<BackendArg>,
        /// Apple's Metal performance HUD (MTL_HUD_ENABLED)
        #[arg(long, value_enum)]
        metal_hud: Option<Toggle>,
        /// Log per-frame Metal HUD statistics for FPS numbers (MTL_HUD_LOG_ENABLED)
        #[arg(long, value_enum)]
        metal_hud_log: Option<Toggle>,
        /// DXVK HUD spec such as fps,frametimes; `off` or `inherit`
        #[arg(long, value_name = "SPEC")]
        dxvk_hud: Option<String>,
        /// Wine synchronization (only on engines built with esync/msync)
        #[arg(long, value_enum)]
        sync: Option<SyncArg>,
        /// CPUs reported to programs, or `inherit` (only on engines that read WINENCPU)
        #[arg(long, value_name = "N")]
        cpu_count: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Summarize FPS from Metal HUD log lines (see `graphics --metal-hud-log`)
    Fps {
        /// Log file containing `metal-HUD:` lines (e.g. logs/steam.log)
        file: PathBuf,
        #[arg(long)]
        json: bool,
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
    /// Wine Mac driver Retina mode (full-resolution rendering on HiDPI screens)
    Retina {
        #[arg(value_enum)]
        state: OnOff,
    },
    /// Game controller back ends of Wine's winebus driver; no options shows them
    Gamepad {
        /// Find controllers through SDL
        #[arg(long, value_enum)]
        sdl: Option<OnOff>,
        /// Use raw IOHID devices
        #[arg(long, value_enum)]
        hidraw: Option<OnOff>,
        /// Present SDL controllers as XInput-style gamepads
        #[arg(long, value_enum)]
        map: Option<OnOff>,
    },
}

/// Bottle that a command operates on and that should receive its log file.
pub fn log_bottle(command: &BottleCommand) -> Option<&str> {
    match command {
        BottleCommand::Ps { id, .. }
        | BottleCommand::Kill { id, .. }
        | BottleCommand::Reg { id, .. }
        | BottleCommand::Preset { id, .. }
        | BottleCommand::Tool { id, .. }
        | BottleCommand::Graphics { id, .. } => Some(id),
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
        BottleCommand::Graphics {
            id,
            appid,
            backend,
            metal_hud,
            metal_hud_log,
            dxvk_hud,
            sync,
            cpu_count,
            json,
        } => {
            let update = SettingsUpdate {
                metal_hud: metal_hud.map(Toggle::update),
                metal_hud_log: metal_hud_log.map(Toggle::update),
                dxvk_hud: dxvk_hud.map(|spec| match spec.as_str() {
                    "inherit" => None,
                    "off" => Some(String::new()),
                    _ => Some(spec),
                }),
                sync: sync.map(|s| match s {
                    SyncArg::None => Some(SyncMode::None),
                    SyncArg::Esync => Some(SyncMode::Esync),
                    SyncArg::Msync => Some(SyncMode::Msync),
                    SyncArg::Inherit => None,
                }),
                cpu_count: cpu_count
                    .map(|n| match n.as_str() {
                        "inherit" => Ok(None),
                        _ => n.parse().map(Some).with_context(|| {
                            format!("--cpu-count expects a number or `inherit`, got {n:?}")
                        }),
                    })
                    .transpose()?,
            };
            let backend = backend.map(GraphicsBackend::from);
            graphics_command(dirs, &id, appid, backend, update, json)?;
        }
        BottleCommand::Fps { file, json } => fps(&file, json)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn graphics_command(
    dirs: &AppDirs,
    id: &str,
    appid: Option<u32>,
    backend: Option<GraphicsBackend>,
    update: SettingsUpdate,
    json: bool,
) -> Result<()> {
    let store = BottleStore::new(dirs);
    let changed = backend.is_some() || !update.is_empty();
    match appid {
        Some(appid) if changed => {
            update_game_profile(
                dirs,
                id,
                appid,
                ProfileUpdate {
                    backend,
                    graphics: update,
                    ..Default::default()
                },
            )?;
        }
        None if changed => {
            let mut bottle = store.load(id)?;
            if let Some(backend) = backend {
                validate_backend(&bottle, backend)?;
                bottle.backend = backend;
            }
            update.apply(&mut bottle.graphics)?;
            store.save(&bottle)?;
            tracing::info!(bottle = id, "bottle graphics settings changed");
        }
        _ => {}
    }

    let bottle = store.load(id)?;
    let engine_id = bottle
        .engine
        .clone()
        .ok_or_else(|| anyhow!("bottle {id} has no engine"))?;
    let engine = EngineGraphics::load(dirs, &engine_id)?;
    let program = appid.and_then(|a| game_profile(&bottle, a));
    if let (Some(appid), None) = (appid, program) {
        println!("No settings for {appid} in {id}; showing the bottle's.");
    }
    let plan = graphics::plan(&bottle, &engine, program)?;
    if json {
        let value = serde_json::json!({
            "bottle": id,
            "appid": appid,
            "engine": engine_id,
            "engine_features": engine.features,
            "d3dmetal_overlay": engine.d3dmetal.as_ref().map(|o| o.dir.clone()),
            "plan": plan,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }
    print_plan(&engine, &plan);
    Ok(())
}

fn print_plan(engine: &EngineGraphics, plan: &GraphicsPlan) {
    let name = |b: GraphicsBackend| match b {
        GraphicsBackend::D3dMetal => "d3dmetal",
        GraphicsBackend::Dxvk => "dxvk",
    };
    let effective = match plan.effective {
        graphics::EffectiveBackend::D3dMetal => "D3DMetal (engine overlay)",
        graphics::EffectiveBackend::Dxvk => "DXVK + MoltenVK",
        graphics::EffectiveBackend::Wined3d => "Wine's built-in Direct3D (wined3d)",
    };
    println!("Backend:       {} -> {effective}", name(plan.requested));
    println!("Wine:          {}", plan.wine.display());
    let s = &plan.settings;
    let opt = |v: Option<bool>| match v {
        Some(true) => "on",
        Some(false) => "off",
        None => "inherit",
    };
    println!(
        "Settings:      metal-hud {}, metal-hud-log {}, dxvk-hud {}, sync {}, cpu-count {}",
        opt(s.metal_hud),
        opt(s.metal_hud_log),
        match s.dxvk_hud.as_deref() {
            None => "inherit",
            Some("") => "off",
            Some(spec) => spec,
        },
        match s.sync {
            None => "inherit",
            Some(SyncMode::None) => "none",
            Some(SyncMode::Esync) => "esync",
            Some(SyncMode::Msync) => "msync",
        },
        s.cpu_count
            .map_or_else(|| "inherit".to_owned(), |n| n.to_string()),
    );
    let pairs = |m: &std::collections::BTreeMap<String, String>| {
        if m.is_empty() {
            "-".to_owned()
        } else {
            m.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(" ")
        }
    };
    println!("Environment:   {}", pairs(&plan.env));
    println!("DLL overrides: {}", pairs(&plan.dll_overrides));
    let f = engine.features;
    let yes = |b: bool| if b { "yes" } else { "no" };
    if f.inspected {
        println!(
            "Engine {}: esync {}, msync {}, CPU count limit {}, Retina mode {}, D3DMetal overlay {}",
            engine.engine_id,
            yes(f.esync),
            yes(f.msync),
            yes(f.cpu_count_override),
            yes(f.retina_mode),
            yes(engine.d3dmetal.is_some()),
        );
    } else {
        println!("Engine {}: could not be inspected", engine.engine_id);
    }
    for w in &plan.warnings {
        println!("warning: {w}");
    }
    for n in &plan.notes {
        println!("note: {n}");
    }
}

fn fps(file: &std::path::Path, json: bool) -> Result<()> {
    let text = std::fs::read(file)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .with_context(|| format!("could not read {}", file.display()))?;
    let Some(summary) = cross_bottle::bench::summarize(&text) else {
        bail!(
            "no `metal-HUD:` lines in {}; enable them with `cross bottle graphics <id> --metal-hud-log on`",
            file.display()
        );
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!(
            "{} s, {} frames: average {:.1} FPS (min {}, max {} per second)",
            summary.seconds, summary.frames, summary.avg_fps, summary.min_fps, summary.max_fps
        );
        println!(
            "mean present interval {:.3}, mean GPU time {:.3} (Metal HUD units)",
            summary.mean_present_interval, summary.mean_gpu_time
        );
    }
    Ok(())
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
        PresetCommand::Retina { state } => {
            runtime.set_retina_mode(state.on())?;
            println!(
                "Retina mode {}; it applies to programs started from now on.",
                if state.on() { "on" } else { "off" }
            );
        }
        PresetCommand::Gamepad { sdl, hidraw, map } => {
            let options = GamepadOptions {
                sdl: sdl.map(OnOff::on),
                hidraw: hidraw.map(OnOff::on),
                map_controllers: map.map(OnOff::on),
            };
            if options != GamepadOptions::default() {
                runtime.set_gamepad_options(&options)?;
                println!(
                    "Saved; restart the bottle (cross bottle kill {id}) for it to take effect."
                );
            }
            let status = runtime.gamepad_status()?;
            let state = |on: bool| if on { "on" } else { "off" };
            println!(
                "SDL {}, IOHID (hidraw) {}, map to XInput {}",
                state(status.sdl),
                state(status.hidraw),
                state(status.map_controllers)
            );
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
    let (bottle, _) = Runtime::open(dirs, &args.bottle)?;
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
    // The bottle's graphics settings; what was given on the command line wins.
    let plan = graphics::plan_for(dirs, &bottle, None)?;
    plan.apply_to(&mut profile);
    for warning in &plan.warnings {
        eprintln!("warning: {warning}");
    }
    let runtime = plan.runtime(dirs, &bottle.id);

    let result = launch::run_with(
        &runtime,
        &LaunchRequest {
            target: args.target,
            args: args.args,
            working_dir: args.workdir,
            profile,
            flags: args.flags,
        },
        &args.options,
    );
    // Recorded only if the user opted in (`cross telemetry enable`); never blocks.
    let telemetry_outcome = match &result {
        Ok(o) if o.status.success() => TelemetryOutcome::Success,
        Ok(_) => TelemetryOutcome::ExitedWithError,
        Err(_) => TelemetryOutcome::FailedToStart,
    };
    // What actually ran, not what was asked for: without GPTK a d3dmetal
    // request falls back to Wine's own Direct3D, which has no backend value.
    let effective = match plan.effective {
        graphics::EffectiveBackend::D3dMetal => Some(GraphicsBackend::D3dMetal),
        graphics::EffectiveBackend::Dxvk => Some(GraphicsBackend::Dxvk),
        graphics::EffectiveBackend::Wined3d => None,
    };
    cross_telemetry::record_launch(dirs, bottle.engine.as_deref(), effective, telemetry_outcome);
    let outcome = result?;
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
