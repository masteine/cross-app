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
    #[error("no crash report {0:?}; see `cross crash list`")]
    CrashReportNotFound(String),
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}

/// One line for an error and its causes, `outer: cause: root`, skipping a cause
/// whose text the previous message already contains. Several of our error types
/// both print their source and return it from `source()`, so a plain chain walk
/// (anyhow's `{:#}`) prints the same text twice.
pub fn chain_message(err: &(dyn std::error::Error + 'static)) -> String {
    let mut out = err.to_string();
    let mut last = out.clone();
    let mut source = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !text.trim().is_empty() && !last.contains(text.trim()) {
            out.push_str(": ");
            out.push_str(&text);
        }
        last = text;
        source = cause.source();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_message_skips_causes_already_printed() {
        let err = Error::io(
            "/x/bottle.toml",
            std::io::Error::new(std::io::ErrorKind::NotFound, "No such file"),
        );
        assert_eq!(
            chain_message(&err),
            "I/O error at /x/bottle.toml: No such file"
        );

        #[derive(Debug, thiserror::Error)]
        #[error("loading failed")]
        struct Outer(#[source] Error);
        let outer = Outer(Error::io("/y", std::io::Error::other("denied")));
        assert_eq!(
            chain_message(&outer),
            "loading failed: I/O error at /y: denied"
        );
    }
}
