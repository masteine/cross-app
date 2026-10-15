//! `cross logs`: find and show log files without digging through the data directory.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::Args;
use cross_bottle::launch::run_logs;
use cross_core::logging::LOG_FILE_PREFIX;
use cross_core::paths::AppDirs;

/// How much of a file is read to find its last lines.
const TAIL_WINDOW: u64 = 2 * 1024 * 1024;

#[derive(Args)]
pub struct LogsArgs {
    /// A bottle's logs instead of the global ones
    #[arg(long)]
    pub bottle: Option<String>,
    /// The latest Wine run log (`cross run` output) of the bottle
    #[arg(long, requires = "bottle")]
    pub wine: bool,
    /// Steam client output of the bottle (`cross steam launch`)
    #[arg(long, requires = "bottle", conflicts_with = "wine")]
    pub steam: bool,
    /// List the log files instead of printing one
    #[arg(long)]
    pub list: bool,
    /// Print only the path of the log file
    #[arg(long)]
    pub path: bool,
    /// Number of lines to print
    #[arg(short = 'n', long, default_value_t = 50)]
    pub lines: usize,
    /// Keep printing new lines as they are written (Ctrl-C to stop)
    #[arg(short, long)]
    pub follow: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Cross,
    Wine,
    Steam,
}

pub fn run(args: &LogsArgs, dirs: &AppDirs) -> Result<ExitCode> {
    let dir = match &args.bottle {
        Some(id) => {
            if !dirs.bottle_file(id).is_file() {
                bail!("bottle {id:?} does not exist; see `cross bottle list`");
            }
            dirs.bottle_logs_dir(id)
        }
        None => dirs.global_logs_dir(),
    };

    if args.list {
        list(&dir)?;
        return Ok(ExitCode::SUCCESS);
    }

    let kind = if args.wine {
        Kind::Wine
    } else if args.steam {
        Kind::Steam
    } else {
        Kind::Cross
    };
    let Some(file) = latest(&dir, kind) else {
        bail!("no {} log yet in {}", kind_name(kind), dir.display());
    };
    if args.path {
        println!("{}", file.display());
        return Ok(ExitCode::SUCCESS);
    }

    let mut out = std::io::stdout().lock();
    for line in tail_lines(&file, args.lines)? {
        writeln!(out, "{line}")?;
    }
    out.flush()?;
    if args.follow {
        follow(&file)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Cross => "cross-app",
        Kind::Wine => "Wine run",
        Kind::Steam => "Steam",
    }
}

/// The newest log of a kind. Daily and run log names sort by time.
fn latest(dir: &Path, kind: Kind) -> Option<PathBuf> {
    match kind {
        Kind::Wine => run_logs(dir).pop(),
        Kind::Steam => Some(dir.join("steam.log")).filter(|p| p.is_file()),
        Kind::Cross => {
            let mut files: Vec<PathBuf> = fs::read_dir(dir)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(LOG_FILE_PREFIX))
                })
                .collect();
            files.sort();
            files.pop()
        }
    }
}

fn list(dir: &Path) -> Result<()> {
    let mut files: Vec<(PathBuf, u64)> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .filter_map(|e| Some((e.path(), e.metadata().ok().filter(|m| m.is_file())?.len())))
            .collect(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    if files.is_empty() {
        println!("No logs yet in {}", dir.display());
        return Ok(());
    }
    files.sort();
    println!("{}", dir.display());
    for (path, size) in files {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_default();
        println!("  {:>10}  {name}", human_size(size));
    }
    Ok(())
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1024 * 1024 => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        b if b >= 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

/// The last `n` lines of a file (reading at most its last few MB).
fn tail_lines(path: &Path, n: usize) -> Result<Vec<String>> {
    let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(TAIL_WINDOW);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // probably cut in the middle
    }
    let skip = lines.len().saturating_sub(n);
    Ok(lines[skip..].iter().map(|l| (*l).to_owned()).collect())
}

/// Print what gets appended to `path` until interrupted.
fn follow(path: &Path) -> Result<()> {
    let mut pos = fs::metadata(path)?.len();
    let mut out = std::io::stdout();
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let len = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if len < pos {
            pos = 0; // truncated or replaced
        }
        if len > pos {
            let mut file = File::open(path)?;
            file.seek(SeekFrom::Start(pos))?;
            let mut chunk = Vec::new();
            file.read_to_end(&mut chunk)?;
            out.write_all(&chunk)?;
            out.flush()?;
            pos += chunk.len() as u64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_newest_file_of_each_kind() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        assert_eq!(latest(dir, Kind::Cross), None);
        for name in [
            "cross.log.2026-10-13",
            "cross.log.2026-10-14",
            "run-20261014-100000.log",
            "run-20261014-120000.log",
            "unrelated.txt",
        ] {
            fs::write(dir.join(name), name).unwrap();
        }
        assert_eq!(
            latest(dir, Kind::Cross).unwrap(),
            dir.join("cross.log.2026-10-14")
        );
        assert_eq!(
            latest(dir, Kind::Wine).unwrap(),
            dir.join("run-20261014-120000.log")
        );
        assert_eq!(latest(dir, Kind::Steam), None);
        fs::write(dir.join("steam.log"), "").unwrap();
        assert_eq!(latest(dir, Kind::Steam).unwrap(), dir.join("steam.log"));
    }

    #[test]
    fn tail_returns_the_last_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("log");
        let text: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        fs::write(&file, text).unwrap();
        assert_eq!(
            tail_lines(&file, 3).unwrap(),
            ["line 98", "line 99", "line 100"]
        );
        assert_eq!(tail_lines(&file, 1000).unwrap().len(), 100);
        assert!(tail_lines(&file, 0).unwrap().is_empty());
    }

    #[test]
    fn sizes_are_human_readable() {
        assert_eq!(human_size(10), "10 B");
        assert_eq!(human_size(2048), "2.0 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
    }
}
