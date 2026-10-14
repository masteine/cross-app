//! Wine engine management: catalog, verified downloads, installation,
//! Game Porting Toolkit import and Rosetta setup.
//!
//! Long operations report progress through a `FnMut(Progress)` callback so any
//! front end (CLI today, GUI later) can render it.

pub mod catalog;
pub mod download;
mod error;
pub mod gptk;
pub mod manager;
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
