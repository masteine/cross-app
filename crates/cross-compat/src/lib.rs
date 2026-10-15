//! Game compatibility profiles.
//!
//! A profile says, for one game (matched by Steam appid, executable SHA-256 or
//! executable name), which backend, sync mode, environment, DLL overrides,
//! arguments and dependencies to use, what is known to break, and an expected
//! rating (green/yellow/red). See [`profile`] for the file format and [`db`]
//! for where profiles come from (built-in, community bundle, local overrides).
//!
//! # API for front ends
//!
//! ```no_run
//! use cross_compat::{CompatDb, Rating};
//! # fn main() -> cross_compat::Result<()> {
//! let dirs = cross_core::paths::AppDirs::discover()?;
//! let db = CompatDb::load(&dirs)?;              // built-in + community + local
//! if let Some(entry) = db.by_appid(367520) {   // or db.by_exe(path)?
//!     let p = &entry.profile;
//!     let colour = match p.rating {
//!         Rating::Green => "green",
//!         Rating::Yellow => "yellow",
//!         Rating::Red => "red",
//!     };
//!     println!("{} {colour} {}", p.name, p.badge_text()); // "green (unverified)"
//!     // p.is_verified(), p.known_bugs, p.anticheat, p.source.url ...
//! }
//! # Ok(()) }
//! ```
//!
//! To use a profile, [`apply_to_bottle`] writes it into a bottle's game entry
//! (the caller saves the bottle and installs [`Applied::missing_deps`]).
//!
//! Every built-in profile is **unverified**: written from public sources, not
//! from running the game with cross-app. A badge must say so.

pub mod db;
mod error;
pub mod profile;

pub use db::{apply_to_bottle, Applied, CompatDb, Entry, Layer, Match, MatchKind};
pub use error::{Error, Result};
pub use profile::{GameProfile, Rating};
