use std::path::PathBuf;

use serde::{Serialize, Serializer};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] cross_core::Error),
    // Boxed: these are large and would bloat every Result.
    #[error(transparent)]
    Engine(Box<cross_engine::Error>),
    #[error(transparent)]
    Bottle(Box<cross_bottle::Error>),
    #[error(transparent)]
    Deps(Box<cross_deps::Error>),
    #[error(transparent)]
    Steam(Box<cross_steam::Error>),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    InvalidInput(String),
    /// A background task panicked or the window went away.
    #[error("{0}")]
    Internal(String),
}

impl From<cross_engine::Error> for Error {
    fn from(e: cross_engine::Error) -> Self {
        Error::Engine(Box::new(e))
    }
}

impl From<cross_bottle::Error> for Error {
    fn from(e: cross_bottle::Error) -> Self {
        Error::Bottle(Box::new(e))
    }
}

impl From<cross_deps::Error> for Error {
    fn from(e: cross_deps::Error) -> Self {
        Error::Deps(Box::new(e))
    }
}

impl From<cross_steam::Error> for Error {
    fn from(e: cross_steam::Error) -> Self {
        Error::Steam(Box::new(e))
    }
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

/// Tauri hands command errors to the page as JSON; the page only needs the message.
impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
