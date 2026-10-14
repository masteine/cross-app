//! Steam (the Windows client) inside a bottle.
//!
//! - [`install`]: download the pinned `SteamSetup.exe`, install it silently, apply patches;
//! - [`patches`]: `steam-patches.toml`, updatable separately from cross-app releases;
//! - [`library`]: installed games from `libraryfolders.vdf` / `appmanifest_*.acf`;
//! - [`launch`]: start Steam or a game (`steam.exe -applaunch <appid>`) with per-game settings;
//! - [`covers`]: cover art from the Steam CDN.

pub mod covers;
mod error;
pub mod install;
pub mod launch;
pub mod library;
pub mod patches;
pub mod vdf;

pub use error::{Error, Result};

/// Where SteamSetup.exe installs the client, as Windows sees it.
pub const STEAM_EXE_WIN: &str = r"C:\Program Files (x86)\Steam\steam.exe";
/// The same folder relative to the Wine prefix.
pub const STEAM_DIR_IN_PREFIX: &str = "drive_c/Program Files (x86)/Steam";
