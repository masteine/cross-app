use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] cross_core::Error),
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid catalog: {0}")]
    InvalidCatalog(String),
    #[error("invalid TOML in {path}: {source}")]
    TomlParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("could not serialize TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error(
        "invalid id {0:?}: use 1-64 characters from [a-z0-9._-], starting with a letter or digit"
    )]
    InvalidId(String),
    #[error("unknown engine {0:?}; see `cross engine list`")]
    UnknownEngine(String),
    #[error("engine {0:?} is already installed")]
    AlreadyInstalled(String),
    #[error("engine {0:?} is not installed")]
    NotInstalled(String),
    #[error("download of {url} failed: {message}")]
    Download { url: String, message: String },
    #[error("checksum mismatch for {label}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        label: String,
        expected: String,
        actual: String,
    },
    #[error("could not extract {archive}: {message}")]
    Extract { archive: PathBuf, message: String },
    #[error("no wine executable found in the extracted engine at {0}")]
    WineNotFound(PathBuf),
    #[error("no `lib/external` directory found in {0}; is this the Game Porting Toolkit image?")]
    GptkNotFound(PathBuf),
    #[error("`{program}` failed: {message}")]
    Command { program: String, message: String },
    #[error("the imported Game Porting Toolkit at {dir} is incomplete (missing: {}); re-import it with `cross engine import-gptk`", .missing.join(", "))]
    GptkIncomplete { dir: PathBuf, missing: Vec<String> },
    #[error("{path} is not a Wine builtin DLL, so it is not Apple's D3DMetal forwarder")]
    NotBuiltinDll { path: PathBuf },
    #[error("engine {0:?} is not known to run D3DMetal (it needs a CrossOver-derived Wine build); pass --force to try anyway")]
    NotGptkCapable(String),
    #[error(
        "engine {engine:?} does not have the expected `bin/../lib/wine/x86_64-windows` layout"
    )]
    UnsupportedLayout { engine: String },
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
