//! `cross steam ...`: Steam inside a bottle.

use std::process::ExitCode;

use anyhow::{anyhow, bail, Result};
use clap::{Subcommand, ValueEnum};
use cross_core::config::{BottleStore, Config};
use cross_core::model::GraphicsBackend;
use cross_core::paths::AppDirs;
use cross_steam::launch::{self, ProfileUpdate};
use cross_steam::library::{self, GameState};
use cross_steam::patches::{self, Source, SteamPatches};
use cross_steam::{covers, install};

use crate::progress::ProgressRenderer;

#[derive(Clone, Copy, ValueEnum)]
pub enum BackendArg {
    Dxvk,
    D3dmetal,
}

impl From<BackendArg> for GraphicsBackend {
    fn from(arg: BackendArg) -> Self {
        match arg {
            BackendArg::Dxvk => GraphicsBackend::Dxvk,
            BackendArg::D3dmetal => GraphicsBackend::D3dMetal,
        }
    }
}

#[derive(Subcommand)]
pub enum SteamCommand {
    /// Download the pinned SteamSetup.exe and install Steam into a bottle
    Install {
        bottle: String,
        /// Reinstall over an existing Steam
        #[arg(long)]
        force: bool,
    },
    /// Start the Steam client (log in there) and wait until it exits
    Run {
        bottle: String,
        /// Extra steam.exe arguments (after `--`)
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// List installed games
    Games {
        bottle: String,
        /// Include Steam tools such as the common redistributables
        #[arg(long)]
        all: bool,
        #[arg(long)]
        json: bool,
    },
    /// Start a game through Steam (returns immediately)
    Launch { bottle: String, appid: u32 },
    /// Download cover art for installed games into the cache
    Covers {
        bottle: String,
        /// Download again even if cached
        #[arg(long)]
        force: bool,
    },
    /// Show or change a game's settings (backend, env, arguments)
    Profile {
        bottle: String,
        appid: u32,
        /// Direct3D 10/11 backend for this game
        #[arg(long, value_enum)]
        backend: Option<BackendArg>,
        /// Environment variable KEY=VALUE (KEY= removes it); repeatable
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Replace the game's arguments (repeatable)
        #[arg(long = "arg", value_name = "ARG", allow_hyphen_values = true)]
        args: Vec<String>,
        /// Remove all settings for this game
        #[arg(long)]
        clear: bool,
    },
    /// Show the Steam patch set; --update fetches a newer one
    Patches {
        #[arg(long)]
        update: bool,
        /// Patch file URL (default: steam_patches_url in config.toml)
        #[arg(long)]
        url: Option<String>,
        /// Accept the fetched file even if its revision is not newer
        #[arg(long)]
        force: bool,
        /// Drop the downloaded patch file and use the built-in one
        #[arg(long, conflicts_with = "update")]
        reset: bool,
    },
}

/// Bottle that should receive this command's log output.
pub fn log_bottle(command: &SteamCommand) -> Option<&str> {
    match command {
        SteamCommand::Install { bottle, .. }
        | SteamCommand::Run { bottle, .. }
        | SteamCommand::Games { bottle, .. }
        | SteamCommand::Launch { bottle, .. }
        | SteamCommand::Covers { bottle, .. }
        | SteamCommand::Profile { bottle, .. } => Some(bottle),
        SteamCommand::Patches { .. } => None,
    }
}

pub fn run(command: SteamCommand, dirs: &AppDirs, config: &Config) -> Result<ExitCode> {
    match command {
        SteamCommand::Install { bottle, force } => {
            eprintln!("Installing Steam into {bottle} ...");
            let mut renderer = ProgressRenderer::new();
            let result = install::install(dirs, &bottle, force, &mut |p| renderer.update(p));
            renderer.finish();
            let report = result?;
            println!(
                "Steam installed (patch revision {}). Start it and log in: cross steam run {bottle}",
                report.patches_revision
            );
        }
        SteamCommand::Run { bottle, args } => {
            let status = launch::run_client(dirs, &bottle, &args)?;
            if !status.success() {
                bail!("Steam exited with {status}");
            }
        }
        SteamCommand::Games { bottle, all, json } => games(dirs, &bottle, all, json)?,
        SteamCommand::Launch { bottle, appid } => {
            let launched = launch::launch_game(dirs, &bottle, appid)?;
            println!(
                "Launching {appid} via Steam (output: {}).",
                launched.log.display()
            );
            for warning in &launched.graphics.warnings {
                eprintln!("warning: {warning}");
            }
            if launched.steam_was_running && launched.has_profile {
                eprintln!(
                    "note: Steam was already running in {bottle}, so this game's settings were not applied; \
                     quit Steam and launch again"
                );
            }
        }
        SteamCommand::Covers { bottle, force } => {
            let prefix = dirs.bottle_prefix(&bottle);
            BottleStore::new(dirs).load(&bottle)?;
            let mut missing = 0;
            for game in library::games(&prefix)?.into_iter().filter(|g| !g.is_tool) {
                match covers::fetch_cover(dirs, game.appid, force)? {
                    Some(path) => println!("{:>8}  {}  {}", game.appid, game.name, path.display()),
                    None => {
                        missing += 1;
                        println!(
                            "{:>8}  {}  (no cover on the Steam CDN)",
                            game.appid, game.name
                        );
                    }
                }
            }
            if missing > 0 {
                eprintln!("{missing} game(s) without cover art");
            }
        }
        SteamCommand::Profile {
            bottle,
            appid,
            backend,
            env,
            args,
            clear,
        } => profile(dirs, &bottle, appid, backend, env, args, clear)?,
        SteamCommand::Patches {
            update,
            url,
            force,
            reset,
        } => patches_command(dirs, config, update, url, force, reset)?,
    }
    Ok(ExitCode::SUCCESS)
}

fn games(dirs: &AppDirs, bottle: &str, all: bool, json: bool) -> Result<()> {
    BottleStore::new(dirs).load(bottle)?;
    let games: Vec<_> = library::games(&dirs.bottle_prefix(bottle))?
        .into_iter()
        .filter(|g| all || !g.is_tool)
        .collect();
    if json {
        let rows: Vec<_> = games
            .iter()
            .map(|g| {
                serde_json::json!({
                    "appid": g.appid,
                    "name": g.name,
                    "state": g.state,
                    "size_on_disk": g.size_on_disk,
                    "install_dir": g.install_dir,
                    "cover": covers::cover_path(dirs, g.appid).is_file()
                        .then(|| covers::cover_path(dirs, g.appid)),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    if games.is_empty() {
        println!("No games installed in {bottle} (or Steam is not installed: cross steam install {bottle}).");
        return Ok(());
    }
    println!("{:>8}  {:15}  {:>9}  NAME", "APPID", "STATE", "SIZE");
    for g in games {
        let state = match g.state {
            GameState::Installed => "installed",
            GameState::UpdateRequired => "update required",
            GameState::Updating => "updating",
            GameState::Other => "incomplete",
        };
        let gib = g.size_on_disk as f64 / (1024.0 * 1024.0 * 1024.0);
        println!("{:>8}  {state:15}  {gib:>6.1} GB  {}", g.appid, g.name);
    }
    Ok(())
}

fn profile(
    dirs: &AppDirs,
    bottle: &str,
    appid: u32,
    backend: Option<BackendArg>,
    env: Vec<String>,
    args: Vec<String>,
    clear: bool,
) -> Result<()> {
    let show_only = backend.is_none() && env.is_empty() && args.is_empty() && !clear;
    if show_only {
        let b = BottleStore::new(dirs).load(bottle)?;
        match launch::game_profile(&b, appid) {
            Some(p) => println!("{}", toml_string(p)?),
            None => println!("No settings for {appid} in {bottle}."),
        }
        return Ok(());
    }
    let mut pairs = Vec::new();
    for item in env {
        let (k, v) = item
            .split_once('=')
            .filter(|(k, _)| !k.is_empty())
            .ok_or_else(|| anyhow!("--env expects KEY=VALUE, got {item:?}"))?;
        pairs.push((k.to_owned(), v.to_owned()));
    }
    // Use the game's real name from its manifest when we know it.
    let name = library::games(&dirs.bottle_prefix(bottle))
        .ok()
        .and_then(|games| games.into_iter().find(|g| g.appid == appid))
        .map(|g| g.name);
    let updated = launch::update_game_profile(
        dirs,
        bottle,
        appid,
        ProfileUpdate {
            name,
            backend: backend.map(GraphicsBackend::from),
            env: pairs,
            args: (!args.is_empty()).then_some(args),
            clear,
            ..Default::default()
        },
    )?;
    match updated {
        Some(app) => println!(
            "Settings for {} ({appid}):\n{}",
            app.name,
            toml_string(&app.profile)?
        ),
        None => println!("Removed settings for {appid}."),
    }
    Ok(())
}

fn toml_string<T: serde::Serialize>(value: &T) -> Result<String> {
    let text = toml::to_string_pretty(value)?;
    Ok(if text.trim().is_empty() {
        "(defaults)".to_owned()
    } else {
        text
    })
}

fn patches_command(
    dirs: &AppDirs,
    config: &Config,
    update: bool,
    url: Option<String>,
    force: bool,
    reset: bool,
) -> Result<()> {
    if reset {
        patches::reset(dirs)?;
        println!("Using the built-in Steam patches again.");
    } else if update {
        let url = url
            .or_else(|| config.steam_patches_url.clone())
            .ok_or_else(|| {
                anyhow!("no patch URL: pass --url or set steam_patches_url in config.toml")
            })?;
        let outcome = patches::update_from(dirs, &url, force)?;
        if outcome.updated {
            println!(
                "Steam patches updated: revision {} -> {}.",
                outcome.from, outcome.to
            );
        } else {
            println!(
                "Already up to date (have revision {}, fetched {}).",
                outcome.from, outcome.to
            );
        }
    }
    let (current, source) = SteamPatches::load(dirs)?;
    let source = match source {
        Source::Builtin => "built-in".to_owned(),
        Source::Local(path) => path.display().to_string(),
    };
    println!("Revision {} ({source})", current.revision);
    println!(
        "Installer: {} (sha256 {})",
        current.installer.url, current.installer.sha256
    );
    println!("steam.exe flags: {}", current.launch.args.join(" "));
    for r in &current.registry {
        println!(
            "Registry: {}\\{} = {:?}  # {}",
            r.key, r.name, r.data, r.reason
        );
    }
    Ok(())
}
