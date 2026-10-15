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
    #[error(
        "engine {id:?} runs x86_64 Wine through Rosetta 2, which is not installed; \
         run `cross engine install-rosetta` or use a native or external engine"
    )]
    RosettaRequired { id: String },
    #[error("engine {id:?} is declared native, but {path} is {found}, not arm64")]
    NotNative {
        id: String,
        path: PathBuf,
        found: String,
    },
    #[error("engine {id:?} needs {requirement}; this Mac has {found}")]
    Unsupported {
        id: String,
        requirement: String,
        found: String,
    },
    #[error("{what} not found (looked in: {})", format_paths(.searched))]
    ExternalNotFound {
        what: String,
        searched: Vec<PathBuf>,
    },
    #[error("engine {id:?} points to {path}, which no longer exists; reinstall or re-register it")]
    ExternalMissing { id: String, path: PathBuf },
}

fn format_paths(paths: &[PathBuf]) -> String {
    if paths.is_empty() {
        return "nowhere".to_owned();
    }
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
