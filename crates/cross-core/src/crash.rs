//! Local crash reports.
//!
//! [`install`] adds a panic hook that writes a plain-text report to
//! `<data>/crashes/<id>.txt`: program version, macOS version, chip, the panic
//! message and location, a backtrace and the last lines of the current log
//! file. Reports never leave the Mac: nothing here touches the network. The user
//! can read them (`cross crash show`) and attach them to a bug report by hand.
//!
//! Setting `CROSS_NO_CRASH_REPORTS=1` turns the hook into a no-op.

use std::backtrace::Backtrace;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::logging::LOG_FILE_PREFIX;
use crate::paths::AppDirs;
use crate::system::SystemInfo;
use crate::{Error, Result};

/// Subdirectory of the data directory that holds the reports.
pub const CRASHES_DIR: &str = "crashes";
/// Environment variable that disables writing crash reports.
pub const DISABLE_ENV: &str = "CROSS_NO_CRASH_REPORTS";
/// Oldest reports are deleted beyond this many.
pub const MAX_REPORTS: usize = 20;
/// How many trailing log lines a report carries.
pub const LOG_TAIL_LINES: usize = 50;
/// Only the end of a large log file is read.
const LOG_TAIL_BYTES: u64 = 64 * 1024;
const FILE_PREFIX: &str = "crash-";
const FILE_EXT: &str = "txt";

/// What the panic hook needs to know about the running program.
#[derive(Debug, Clone)]
pub struct CrashContext {
    /// Where reports are written (`<data>/crashes`).
    pub crashes_dir: PathBuf,
    /// Directory of the log file that is being written, for the log tail.
    pub log_dir: Option<PathBuf>,
    /// Probed for the `gptk` field of [`SystemInfo`]; not otherwise used.
    pub gptk_dir: PathBuf,
    /// `cross`, `cross-gui`, ...
    pub program: String,
    pub version: String,
}

impl CrashContext {
    pub fn new(dirs: &AppDirs, log_dir: Option<PathBuf>, program: &str, version: &str) -> Self {
        Self {
            crashes_dir: crashes_dir(dirs),
            log_dir,
            gptk_dir: dirs.gptk_dir(),
            program: program.to_owned(),
            version: version.to_owned(),
        }
    }
}

/// A saved report, as listed by [`list`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrashReport {
    /// File stem, e.g. `crash-1792001217-4242`; what `show` takes.
    pub id: String,
    pub path: PathBuf,
    /// Seconds since the Unix epoch, from the id.
    pub created_unix: u64,
    /// The panic message (first line), if the report could be read.
    pub message: Option<String>,
}

/// `<data>/crashes`.
pub fn crashes_dir(dirs: &AppDirs) -> PathBuf {
    dirs.root().join(CRASHES_DIR)
}

/// Install the crash-report panic hook. The previous hook (Rust's default
/// message on stderr) still runs first; afterwards the report path is printed.
pub fn install(context: CrashContext) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        previous(info);
        if std::env::var_os(DISABLE_ENV).is_some_and(|v| !v.is_empty() && v != "0") {
            return;
        }
        let message = payload_message(info.payload());
        let location = info.location().map(|l| l.to_string());
        let backtrace = Backtrace::force_capture().to_string();
        match write_report(&context, &message, location.as_deref(), &backtrace) {
            Ok(path) => eprintln!(
                "A crash report was saved to {} (it stays on this Mac; nothing is uploaded).\n\
                 Please attach it to a bug report: `cross crash show` prints it.",
                path.display()
            ),
            Err(err) => eprintln!("could not save a crash report: {err}"),
        }
    }));
}

fn payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "(panic payload is not a string)".to_owned()
    }
}

/// Render and save one report, then prune old ones. Returns the report's path.
pub fn write_report(
    context: &CrashContext,
    message: &str,
    location: Option<&str>,
    backtrace: &str,
) -> Result<PathBuf> {
    let now = now_unix();
    let id = format!("{FILE_PREFIX}{now}-{}", std::process::id());
    let system = SystemInfo::detect(&context.gptk_dir);
    let log_tail = match &context.log_dir {
        Some(dir) => latest_log(dir).map(|p| (tail_lines(&p, LOG_TAIL_LINES), p)),
        None => None,
    };
    let text = render(&Rendered {
        id: &id,
        unix: now,
        context,
        system: &system,
        thread: std::thread::current().name().unwrap_or("<unnamed>"),
        message,
        location,
        backtrace,
        log_tail: log_tail
            .as_ref()
            .map(|(lines, path)| (lines.as_slice(), path.as_path())),
    });

    let dir = &context.crashes_dir;
    fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let path = dir.join(format!("{id}.{FILE_EXT}"));
    fs::write(&path, text).map_err(|e| Error::io(&path, e))?;
    prune(dir, MAX_REPORTS)?;
    Ok(path)
}

struct Rendered<'a> {
    id: &'a str,
    unix: u64,
    context: &'a CrashContext,
    system: &'a SystemInfo,
    thread: &'a str,
    message: &'a str,
    location: Option<&'a str>,
    backtrace: &'a str,
    log_tail: Option<(&'a [String], &'a Path)>,
}

fn render(r: &Rendered<'_>) -> String {
    let unknown = || "unknown".to_owned();
    let mut out = String::new();
    let mut line = |key: &str, value: &str| out.push_str(&format!("{key:<9} {value}\n"));
    line(
        "report:",
        "cross-app crash report (local only, never uploaded)",
    );
    line("id:", r.id);
    line(
        "time:",
        &format!("{} (unix {})", format_utc(r.unix), r.unix),
    );
    line(
        "program:",
        &format!("{} {}", r.context.program, r.context.version),
    );
    line(
        "macOS:",
        &r.system.macos.map_or_else(unknown, |v| v.to_string()),
    );
    line(
        "chip:",
        &r.system
            .chip
            .as_ref()
            .map(ToString::to_string)
            .or_else(|| r.system.cpu_brand.clone())
            .unwrap_or_else(unknown),
    );
    line("arch:", &r.system.arch);
    line(
        "rosetta:",
        if r.system.rosetta_installed {
            "installed"
        } else {
            "not installed"
        },
    );
    line("thread:", r.thread);
    line("location:", r.location.unwrap_or("unknown"));
    // The message may span lines; the first one is what `list` shows.
    line("message:", r.message);
    out.push_str("\nbacktrace:\n");
    out.push_str(r.backtrace.trim_end());
    out.push('\n');
    match r.log_tail {
        Some((lines, path)) => {
            out.push_str(&format!(
                "\nlast {} log lines ({}):\n",
                lines.len(),
                path.display()
            ));
            for l in lines {
                out.push_str(l);
                out.push('\n');
            }
        }
        None => out.push_str("\nno log file found\n"),
    }
    out
}

/// The most recently modified `cross.log*` file in `dir`.
fn latest_log(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(LOG_FILE_PREFIX))
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max()
        .map(|(_, path)| path)
}

/// The last `n` lines of a text file (reading at most its final 64 KiB).
/// Lines the log writer has not flushed yet are not included.
fn tail_lines(path: &Path, n: usize) -> Vec<String> {
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map_or(0, |m| m.len());
    let start = len.saturating_sub(LOG_TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // probably cut in the middle
    }
    let skip = lines.len().saturating_sub(n);
    lines[skip..].iter().map(|l| (*l).to_owned()).collect()
}

/// Seconds since the epoch encoded in an id like `crash-1792001217-4242`.
fn parse_id(id: &str) -> Option<u64> {
    let rest = id.strip_prefix(FILE_PREFIX)?;
    let (secs, pid) = rest.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (digits(secs) && digits(pid)).then(|| secs.parse().ok())?
}

/// All reports, newest first.
pub fn list(dirs: &AppDirs) -> Result<Vec<CrashReport>> {
    list_in(&crashes_dir(dirs))
}

fn list_in(dir: &Path) -> Result<Vec<CrashReport>> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(dir, e)),
    };
    let mut reports: Vec<CrashReport> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some(FILE_EXT) {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_owned();
            let created_unix = parse_id(&id)?;
            let message = fs::read_to_string(&path).ok().and_then(|text| {
                text.lines()
                    .find_map(|l| l.strip_prefix("message:"))
                    .map(|m| m.trim().to_owned())
            });
            Some(CrashReport {
                id,
                path,
                created_unix,
                message,
            })
        })
        .collect();
    reports.sort_by(|a, b| {
        b.created_unix
            .cmp(&a.created_unix)
            .then_with(|| b.id.cmp(&a.id))
    });
    Ok(reports)
}

/// The text of one report; `id` may also be `latest`.
pub fn read(dirs: &AppDirs, id: &str) -> Result<(CrashReport, String)> {
    let reports = list(dirs)?;
    let report = if id == "latest" {
        reports.into_iter().next()
    } else {
        reports.into_iter().find(|r| r.id == id)
    }
    .ok_or_else(|| Error::CrashReportNotFound(id.to_owned()))?;
    let text = fs::read_to_string(&report.path).map_err(|e| Error::io(&report.path, e))?;
    Ok((report, text))
}

/// Delete every report; returns how many were removed.
pub fn clear(dirs: &AppDirs) -> Result<usize> {
    let reports = list(dirs)?;
    for r in &reports {
        fs::remove_file(&r.path).map_err(|e| Error::io(&r.path, e))?;
    }
    Ok(reports.len())
}

/// Keep only the newest `keep` reports.
fn prune(dir: &Path, keep: usize) -> Result<()> {
    for old in list_in(dir)?.into_iter().skip(keep) {
        fs::remove_file(&old.path).map_err(|e| Error::io(&old.path, e))?;
    }
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `2026-10-14T20:46:31Z` for a Unix timestamp (proleptic Gregorian, UTC).
pub fn format_utc(unix: u64) -> String {
    let days = unix / 86_400;
    let secs = unix % 86_400;
    // Howard Hinnant's civil_from_days, for days since 1970-01-01.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(root: &Path) -> CrashContext {
        let dirs = AppDirs::with_root(root);
        CrashContext::new(&dirs, Some(dirs.global_logs_dir()), "cross", "9.9.9")
    }

    #[test]
    fn formats_utc_timestamps() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_utc(1_792_001_217), "2026-10-14T18:06:57Z");
    }

    #[test]
    fn ids_must_be_well_formed() {
        assert_eq!(parse_id("crash-1792001217-42"), Some(1_792_001_217));
        for bad in [
            "crash-",
            "crash-12",
            "crash-x-1",
            "crash-1-",
            "other-1-2",
            "../crash-1-2",
        ] {
            assert_eq!(parse_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn report_carries_system_message_backtrace_and_log_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let ctx = context(tmp.path());
        let logs = ctx.log_dir.clone().unwrap();
        fs::create_dir_all(&logs).unwrap();
        let lines: Vec<String> = (0..80).map(|i| format!("log line {i}")).collect();
        fs::write(logs.join("cross.log.2026-10-14"), lines.join("\n")).unwrap();

        let path =
            write_report(&ctx, "boom\nsecond line", Some("src/x.rs:1:2"), "0: frame").unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("program:  cross 9.9.9"), "{text}");
        assert!(text.contains("message:  boom"), "{text}");
        assert!(text.contains("location: src/x.rs:1:2"), "{text}");
        assert!(text.contains("arch:"), "{text}");
        assert!(text.contains("0: frame"), "{text}");
        assert!(text.contains("log line 79"), "{text}");
        assert!(
            text.contains("log line 30") && !text.contains("log line 29\n"),
            "{text}"
        );

        let dirs = AppDirs::with_root(tmp.path());
        let listed = list(&dirs).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].message.as_deref(), Some("boom"));
        assert_eq!(listed[0].path, path);
        let (latest, body) = read(&dirs, "latest").unwrap();
        assert_eq!(latest.id, listed[0].id);
        assert_eq!(body, text);
        assert!(matches!(
            read(&dirs, "crash-1-1").unwrap_err(),
            Error::CrashReportNotFound(_)
        ));
        assert!(matches!(
            read(&dirs, "../config").unwrap_err(),
            Error::CrashReportNotFound(_)
        ));
    }

    #[test]
    fn missing_log_and_crash_dir_are_fine() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        assert!(list(&dirs).unwrap().is_empty());
        assert_eq!(clear(&dirs).unwrap(), 0);
        let path = write_report(&context(tmp.path()), "x", None, "").unwrap();
        assert!(fs::read_to_string(path)
            .unwrap()
            .contains("no log file found"));
    }

    #[test]
    fn listing_is_newest_first_and_old_reports_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join(CRASHES_DIR);
        fs::create_dir_all(&dir).unwrap();
        for i in 0..25u64 {
            fs::write(
                dir.join(format!("crash-{}-1.txt", 1000 + i)),
                "message: m\n",
            )
            .unwrap();
        }
        fs::write(dir.join("notes.txt"), "").unwrap();
        fs::write(dir.join("crash-5-1.log"), "").unwrap();

        let all = list_in(&dir).unwrap();
        assert_eq!(all.len(), 25);
        assert_eq!(all[0].id, "crash-1024-1");
        prune(&dir, MAX_REPORTS).unwrap();
        let kept = list_in(&dir).unwrap();
        assert_eq!(kept.len(), MAX_REPORTS);
        assert_eq!(kept.last().unwrap().id, "crash-1005-1");
        assert!(dir.join("notes.txt").exists(), "unrelated files stay");

        let dirs = AppDirs::with_root(tmp.path());
        assert_eq!(clear(&dirs).unwrap(), MAX_REPORTS);
        assert!(list(&dirs).unwrap().is_empty());
    }

    /// Single test that touches the process-wide panic hook.
    #[test]
    fn panic_hook_writes_a_report() {
        let tmp = tempfile::tempdir().unwrap();
        install(context(tmp.path()));
        let result = std::panic::catch_unwind(|| panic!("hook test {}", 42));
        let _ = std::panic::take_hook(); // back to the default hook
        assert!(result.is_err());

        let reports = list(&AppDirs::with_root(tmp.path())).unwrap();
        assert_eq!(reports.len(), 1, "{reports:?}");
        assert_eq!(reports[0].message.as_deref(), Some("hook test 42"));
        let text = fs::read_to_string(&reports[0].path).unwrap();
        assert!(
            text.contains("crash.rs"),
            "location points at this file: {text}"
        );
    }
}
