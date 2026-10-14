use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] cross_core::Error),
    // Boxed: these are large and would bloat every Result.
    #[error(transparent)]
    Bottle(Box<cross_bottle::Error>),
    #[error(transparent)]
    Engine(Box<cross_engine::Error>),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid verb catalog: {0}")]
    InvalidCatalog(String),
    #[error("invalid TOML in {path}: {source}")]
    TomlParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("unknown verb {verb:?}{hint}")]
    UnknownVerb { verb: String, hint: String },
    #[error("invalid verb name {0:?}: use [a-z0-9._-], not starting with '-' or '.'")]
    InvalidVerbName(String),
    #[error("{verb} conflicts with {other}, which is already installed in this bottle")]
    Conflict { verb: String, other: String },
    #[error("{verb}: {message}")]
    Step { verb: String, message: String },
    #[error("winetricks failed: {0}")]
    Winetricks(String),
}

impl From<cross_bottle::Error> for Error {
    fn from(e: cross_bottle::Error) -> Self {
        Error::Bottle(Box::new(e))
    }
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

    pub(crate) fn step(verb: &str, message: impl Into<String>) -> Self {
        Error::Step {
            verb: verb.to_owned(),
            message: message.into(),
        }
    }
}
