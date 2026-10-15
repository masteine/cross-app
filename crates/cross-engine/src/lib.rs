//! Wine engine management: catalog, verified downloads, installation,
//! Game Porting Toolkit import and Rosetta setup.
//!
//! An engine's [`cross_core::model::Translation`] drives how it is installed
//! and run (see [`policy`]): Rosetta and native engines are downloaded,
//! external ones (CrossOver, see [`crossover`]) are found on the system.
//!
//! Long operations report progress through a `FnMut(Progress)` callback so any
//! front end (CLI today, GUI later) can render it.

pub mod catalog;
pub mod crossover;
pub mod d3dmetal;
pub mod download;
mod error;
pub mod features;
pub mod gptk;
pub mod macho;
pub mod manager;
pub mod policy;
pub mod rosetta;

pub use error::{Error, Result};

/// A step reported while installing something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress<'a> {
    Downloading {
        label: &'a str,
        done: u64,
        total: Option<u64>,
    },
    /// The archive was already in the download cache and passed verification.
    CacheHit {
        label: &'a str,
    },
    Verifying {
        label: &'a str,
    },
    Extracting {
        label: &'a str,
    },
}
