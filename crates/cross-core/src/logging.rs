//! `tracing` setup: human-readable stderr plus a daily-rolling log file.
//!
//! Commands that operate on a bottle log into `bottles/<id>/logs/`, everything
//! else into the global `logs/` directory.

use std::path::PathBuf;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

use crate::paths::AppDirs;
use crate::{Error, Result};

/// File name prefix; the appender adds the date (`cross.log.2026-10-14`).
pub const LOG_FILE_PREFIX: &str = "cross.log";

#[derive(Debug, Clone, Copy)]
pub enum LogTarget<'a> {
    Global,
    Bottle(&'a str),
}

impl LogTarget<'_> {
    pub fn dir(&self, dirs: &AppDirs) -> PathBuf {
        match self {
            LogTarget::Global => dirs.global_logs_dir(),
            LogTarget::Bottle(id) => dirs.bottle_logs_dir(id),
        }
    }
}

/// Keeps the background log writer alive; drop it to flush pending lines.
#[must_use = "dropping the guard stops file logging"]
pub struct LogGuard {
    _worker: WorkerGuard,
}

/// Install the global subscriber. `RUST_LOG` takes precedence over `default_filter`.
/// Fails if a subscriber is already installed.
pub fn init(dirs: &AppDirs, target: LogTarget<'_>, default_filter: &str) -> Result<LogGuard> {
    let log_dir = target.dir(dirs);
    std::fs::create_dir_all(&log_dir).map_err(|e| Error::io(&log_dir, e))?;

    let appender = tracing_appender::rolling::daily(&log_dir, LOG_FILE_PREFIX);
    let (file_writer, worker) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(default_filter))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false).with_writer(std::io::stderr))
        .with(fmt::layer().with_ansi(false).with_writer(file_writer))
        .try_init()
        .map_err(|e| Error::Logging(e.to_string()))?;

    Ok(LogGuard { _worker: worker })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Single test: the global subscriber can only be installed once per process.
    #[test]
    fn bottle_logs_land_in_the_bottle_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());

        let guard = init(&dirs, LogTarget::Bottle("demo"), "info").unwrap();
        tracing::info!("hello from the bottle");
        drop(guard); // flush

        let logs = dirs.bottle_logs_dir("demo");
        let file = std::fs::read_dir(&logs)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(LOG_FILE_PREFIX))
            })
            .expect("log file created");
        let text = std::fs::read_to_string(file).unwrap();
        assert!(text.contains("hello from the bottle"), "{text}");
        assert!(!dirs.global_logs_dir().exists());
    }
}
