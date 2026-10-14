use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not determine the user's home directory")]
    NoHomeDir,
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid TOML in {path}: {source}")]
    TomlParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("could not serialize TOML: {0}")]
    TomlSerialize(#[from] toml::ser::Error),
    #[error("invalid bottle id {0:?}: use 1-64 characters from [a-z0-9_-], starting with a letter or digit")]
    InvalidBottleId(String),
    #[error("could not initialise logging: {0}")]
    Logging(String),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
