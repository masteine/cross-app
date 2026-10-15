//! Bottles: isolated Wine prefixes with their own settings.
//!
//! This crate creates and manages bottles, runs programs inside them, lists and
//! stops their processes, and edits their registry. It drives the Wine binary of
//! an engine installed by `cross-engine`; it never talks to a UI.

pub mod bench;
pub mod crossover;
mod error;
pub mod exec;
pub mod graphics;
pub mod input;
pub mod launch;
pub mod manager;
pub mod presets;
pub mod processes;
pub mod registry;
pub mod runtime;

pub use error::{Error, Result};
pub use runtime::Runtime;
