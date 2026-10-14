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
    #[error("bottle {0:?} already exists")]
    BottleExists(String),
    #[error("bottle {0:?} does not exist; see `cross bottle list`")]
    BottleNotFound(String),
    #[error("no engine chosen and none can be picked automatically (installed: {})", format_list(.installed))]
    NoEngineChosen { installed: Vec<String> },
    #[error("bottle {bottle:?} has no engine; recreate it with --engine")]
    BottleHasNoEngine { bottle: String },
    #[error("engine {0:?} is not installed; see `cross engine list`")]
    EngineNotInstalled(String),
    #[error("bottle {bottle:?} is running ({count} process(es)); stop it first with `cross bottle kill {bottle}`")]
    BottleRunning { bottle: String, count: usize },
    #[error("{what} failed: {message}")]
    Wine { what: String, message: String },
    #[error("{what} did not finish within {seconds} s")]
    Timeout { what: String, seconds: u64 },
    #[error("{0}")]
    InvalidInput(String),
}

fn format_list(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_owned()
    } else {
        items.join(", ")
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
