//! The command layer: plain Rust over the cross-* crates, returning
//! serializable views. Tauri commands in [`crate::app`] are thin wrappers
//! around these functions, which keeps them testable without a window.

pub mod apps;
pub mod bottles;
pub mod compat;
pub mod engines;
pub mod library;
pub mod logs;
pub mod progress;
pub mod system;

#[cfg(test)]
pub(crate) mod test_support;
