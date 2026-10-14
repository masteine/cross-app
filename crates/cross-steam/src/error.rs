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
    #[error("{path}: invalid VDF at line {line}: {message}")]
    Vdf {
        path: PathBuf,
        line: usize,
        message: String,
    },
    #[error("invalid steam-patches.toml: {0}")]
    InvalidPatches(String),
    #[error("Steam is already installed in bottle {0:?} (use --force to reinstall)")]
    AlreadyInstalled(String),
    #[error("Steam is not installed in bottle {0:?}; run `cross steam install {0}`")]
    NotInstalled(String),
    #[error("Steam setup failed: {0}")]
    Setup(String),
    #[error("{0}")]
    InvalidInput(String),
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
}
