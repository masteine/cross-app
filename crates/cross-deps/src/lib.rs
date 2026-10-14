//! Windows dependencies for bottles, in the spirit of winetricks but declarative:
//! each verb is data (pinned downloads plus steps) in `verbs.toml`, not a script.
//!
//! Installers are downloaded once into the shared cache, verified by SHA-256, and
//! installed offline from then on. What is installed is recorded in `bottle.toml`.

pub mod catalog;
mod error;
pub mod install;
pub mod winetricks;

pub use error::{Error, Result};
