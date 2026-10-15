//! `tracing` setup: human-readable stderr plus a daily-rolling log file.
//!
//! Commands that operate on a bottle log into `bottles/<id>/logs/`, everything
//! else into the global `logs/` directory. The two outputs are filtered
//! separately: the file keeps the full detail needed to debug a run afterwards,
//! while the terminal only shows what the user asked for (`-v`, `-q`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

use crate::paths::AppDirs;
use crate::{Error, Result};

/// File name prefix; the appender adds the date (`cross.log.2026-10-14`).
pub const LOG_FILE_PREFIX: &str = "cross.log";

/// Daily log files kept per directory; older ones are deleted on rotation.
pub const MAX_LOG_FILES: usize = 14;

/// Directives that keep HTTP/TLS internals out of debug logs.
pub const QUIET_DEPENDENCIES: &str = "hyper=info,hyper_util=info,reqwest=info,rustls=info,h2=info";

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

/// Filters for the two outputs, in `EnvFilter` syntax (`debug`, `warn,cross_steam=debug`).
/// `RUST_LOG`, when set, replaces both.
#[derive(Debug, Clone, Copy)]
pub struct LogOptions<'a> {
    pub file_filter: &'a str,
    pub stderr_filter: &'a str,
}

/// Keeps the background log writer alive; drop it to flush pending lines.
#[must_use = "dropping the guard stops file logging"]
pub struct LogGuard {
    _worker: WorkerGuard,
    /// Directory the log file is written to.
    pub dir: PathBuf,
}

/// Install the global subscriber with the same filter for both outputs.
/// `RUST_LOG` takes precedence over `default_filter`. Fails if a subscriber is
/// already installed.
pub fn init(dirs: &AppDirs, target: LogTarget<'_>, default_filter: &str) -> Result<LogGuard> {
    init_with(
        dirs,
        target,
        &LogOptions {
            file_filter: default_filter,
            stderr_filter: default_filter,
        },
    )
}

/// Install the global subscriber with separate file and terminal filters.
pub fn init_with(
    dirs: &AppDirs,
    target: LogTarget<'_>,
    options: &LogOptions<'_>,
) -> Result<LogGuard> {
    let log_dir = target.dir(dirs);
    std::fs::create_dir_all(&log_dir).map_err(|e| Error::io(&log_dir, e))?;

    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .max_log_files(MAX_LOG_FILES)
        .build(&log_dir)
        .map_err(|e| Error::Logging(e.to_string()))?;
    let (file_writer, worker) = tracing_appender::non_blocking(appender);

    let filter = |directives: &str| {
        EnvFilter::try_from_default_env()
            .or_else(|_| EnvFilter::try_new(directives))
            .unwrap_or_else(|_| EnvFilter::new("info"))
    };

    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_target(false)
                .with_writer(std::io::stderr)
                .with_filter(filter(options.stderr_filter)),
        )
        .with(
            fmt::layer()
                .with_ansi(false)
                .fmt_fields(PlainFields::default())
                .with_writer(file_writer)
                .with_filter(filter(options.file_filter)),
        )
        .try_init()
        .map_err(|e| Error::Logging(e.to_string()))?;

    Ok(LogGuard {
        _worker: worker,
        dir: log_dir,
    })
}

/// Field formatter for the file. A type of its own matters: `tracing-subscriber`
/// caches span fields per formatter type, so sharing `DefaultFields` with the
/// coloured terminal layer would put ANSI escape codes into the file.
#[derive(Default)]
struct PlainFields(fmt::format::DefaultFields);

impl<'writer> fmt::FormatFields<'writer> for PlainFields {
    fn format_fields<R: tracing_subscriber::field::RecordFields>(
        &self,
        writer: fmt::format::Writer<'writer>,
        fields: R,
    ) -> std::fmt::Result {
        self.0.format_fields(writer, fields)
    }
}

/// `2026-10-14T19:30:05Z` (UTC) for log headers.
pub fn timestamp_utc(time: SystemTime) -> String {
    let (date, (h, m, s)) = split_utc(time);
    format!(
        "{}-{:02}-{:02}T{h:02}:{m:02}:{s:02}Z",
        date.0, date.1, date.2
    )
}

/// `20261014-193005` (UTC) for file names: sortable, no separators that need quoting.
pub fn timestamp_compact(time: SystemTime) -> String {
    let (date, (h, m, s)) = split_utc(time);
    format!("{}{:02}{:02}-{h:02}{m:02}{s:02}", date.0, date.1, date.2)
}

type Ymd = (i64, u32, u32);
type Hms = (u64, u64, u64);

fn split_utc(time: SystemTime) -> (Ymd, Hms) {
    let secs = time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    (
        civil_from_days(days),
        (rem / 3600, rem % 3600 / 60, rem % 60),
    )
}

/// Gregorian date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> Ymd {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Single test: the global subscriber can only be installed once per process.
    #[test]
    fn file_and_terminal_are_filtered_separately() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());

        let guard = init_with(
            &dirs,
            LogTarget::Bottle("demo"),
            &LogOptions {
                file_filter: "debug",
                stderr_filter: "error",
            },
        )
        .unwrap();
        assert_eq!(guard.dir, dirs.bottle_logs_dir("demo"));
        tracing::info!("hello from the bottle");
        tracing::debug!("detail only for the file");
        tracing::trace!("too fine for both");
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
        assert!(text.contains("detail only for the file"), "{text}");
        assert!(!text.contains("too fine"), "{text}");
        assert!(!dirs.global_logs_dir().exists());
    }

    #[test]
    fn timestamps() {
        let t = UNIX_EPOCH + Duration::from_secs(1_792_005_182);
        assert_eq!(timestamp_utc(t), "2026-10-14T19:13:02Z");
        assert_eq!(timestamp_compact(t), "20261014-191302");
        assert_eq!(timestamp_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        // Leap day and end of year.
        assert_eq!(
            timestamp_utc(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
        assert_eq!(
            timestamp_utc(UNIX_EPOCH + Duration::from_secs(1_798_761_599)),
            "2026-12-31T23:59:59Z"
        );
    }
}
