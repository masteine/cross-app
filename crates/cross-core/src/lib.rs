//! Domain model, configuration, logging and environment detection for cross-app.
//!
//! This crate knows nothing about any UI: everything the GUI does must be
//! reachable from the CLI through the same API.

pub mod config;
pub mod doctor;
pub mod error;
pub mod logging;
pub mod model;
pub mod paths;
pub mod system;

pub use error::{Error, Result};
