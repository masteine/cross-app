//! The cross-app desktop GUI (Tauri 2).
//!
//! [`api`] holds everything the window can do as plain Rust over the cross-*
//! crates; [`app`] wires it to Tauri (commands, events, menu, Dock progress,
//! notifications). The front end is static HTML/CSS/JS in `ui/`; see
//! `docs/adr/0002-gui-frontend.md`.

pub mod api;
mod app;
mod error;

pub use app::run;
pub use error::{Error, Result};
