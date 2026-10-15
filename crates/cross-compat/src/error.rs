use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] cross_core::Error),
    // Boxed: cross_engine::Error is large and would bloat every Result.
    #[error(transparent)]
    Engine(Box<cross_engine::Error>),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid compatibility profile {origin}: {message}")]
    InvalidProfile { origin: String, message: String },
    #[error("could not serialize TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("no compatibility profile {0:?}; see `cross compat list`")]
    UnknownProfile(String),
    #[error("{0}")]
    InvalidInput(String),
}

impl From<cross_engine::Error> for Error {
    fn from(e: cross_engine::Error) -> Self {
        Error::Engine(Box::new(e))
    }
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn invalid(origin: impl Into<String>, message: impl Into<String>) -> Self {
        Error::InvalidProfile {
            origin: origin.into(),
            message: message.into(),
        }
    }
}
