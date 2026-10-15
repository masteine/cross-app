//! Real-time logs: list the files in a bottle's `logs/` (or the global `logs/`)
//! and read them incrementally, like `tail -f`. The page polls with the offset
//! it got last time, so no file watcher or extra thread is needed.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use cross_core::model::validate_bottle_id;
use cross_core::paths::AppDirs;
use serde::Serialize;

use crate::{Error, Result};

/// Most bytes returned by one read; the first read starts this far from the end.
pub const MAX_CHUNK: u64 = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogFile {
    pub name: String,
    pub size: u64,
    pub modified_unix: u64,
}

fn logs_dir(dirs: &AppDirs, bottle: Option<&str>) -> Result<PathBuf> {
    match bottle {
        Some(id) => {
            validate_bottle_id(id)?;
            Ok(dirs.bottle_logs_dir(id))
        }
        None => Ok(dirs.global_logs_dir()),
    }
}

/// Log files, most recently written first. A missing directory is just empty.
pub fn list(dirs: &AppDirs, bottle: Option<&str>) -> Result<Vec<LogFile>> {
    let dir = logs_dir(dirs, bottle)?;
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(Error::io(&dir, e)),
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() || name.starts_with('.') {
            continue;
        }
        let modified_unix = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        files.push(LogFile {
            name,
            size: meta.len(),
            modified_unix,
        });
    }
    files.sort_by(|a, b| {
        b.modified_unix
            .cmp(&a.modified_unix)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(files)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LogChunk {
    pub text: String,
    /// Pass this back as `offset` on the next read.
    pub offset: u64,
    pub size: u64,
    /// The file shrank (rotated or truncated) or this is the first read: the
    /// page should replace what it shows instead of appending.
    pub reset: bool,
}

/// Read what was appended since `offset`. `None` starts near the end of the file.
pub fn read(
    dirs: &AppDirs,
    bottle: Option<&str>,
    name: &str,
    offset: Option<u64>,
) -> Result<LogChunk> {
    // Only plain file names from `list`: no paths, no hidden files.
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
        return Err(Error::InvalidInput(format!(
            "invalid log file name {name:?}"
        )));
    }
    let path = logs_dir(dirs, bottle)?.join(name);
    let mut file = File::open(&path).map_err(|e| Error::io(&path, e))?;
    let size = file.metadata().map_err(|e| Error::io(&path, e))?.len();

    let (start, reset) = match offset {
        Some(o) if o <= size => (o, false),
        _ => (size.saturating_sub(MAX_CHUNK), true),
    };
    let len = (size - start).min(MAX_CHUNK);
    let mut buf = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    file.seek(SeekFrom::Start(start))
        .map_err(|e| Error::io(&path, e))?;
    file.take(len)
        .read_to_end(&mut buf)
        .map_err(|e| Error::io(&path, e))?;

    let mut skip = 0;
    if reset && start > 0 {
        // Started mid-file: drop the partial first line.
        skip = buf.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1);
    }
    let mut end = buf.len();
    if len == MAX_CHUNK && start + len < size {
        // More to come: stop at a line end so no UTF-8 sequence is split.
        if let Some(i) = buf.iter().rposition(|b| *b == b'\n') {
            end = (i + 1).max(skip);
        }
    }
    Ok(LogChunk {
        text: String::from_utf8_lossy(&buf[skip..end]).into_owned(),
        offset: start + end as u64,
        size,
        reset,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn setup() -> (tempfile::TempDir, AppDirs) {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        fs::create_dir_all(dirs.bottle_logs_dir("demo")).unwrap();
        (tmp, dirs)
    }

    #[test]
    fn tails_appended_lines() {
        let (_tmp, dirs) = setup();
        let path = dirs.bottle_logs_dir("demo").join("game.log");
        fs::write(&path, "one\ntwo\n").unwrap();

        let first = read(&dirs, Some("demo"), "game.log", None).unwrap();
        assert_eq!((first.text.as_str(), first.reset), ("one\ntwo\n", true));

        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(f, "three\npart").unwrap();
        let next = read(&dirs, Some("demo"), "game.log", Some(first.offset)).unwrap();
        assert_eq!((next.text.as_str(), next.reset), ("three\npart", false));

        let idle = read(&dirs, Some("demo"), "game.log", Some(next.offset)).unwrap();
        assert_eq!(idle.text, "");
        assert_eq!(idle.offset, next.offset);
    }

    #[test]
    fn truncated_files_start_over() {
        let (_tmp, dirs) = setup();
        let path = dirs.bottle_logs_dir("demo").join("steam.log");
        fs::write(&path, "a long first session\n").unwrap();
        let first = read(&dirs, Some("demo"), "steam.log", None).unwrap();
        fs::write(&path, "new\n").unwrap();
        let next = read(&dirs, Some("demo"), "steam.log", Some(first.offset)).unwrap();
        assert!(next.reset);
        assert_eq!(next.text, "new\n");
    }

    #[test]
    fn large_files_are_read_in_whole_lines() {
        let (_tmp, dirs) = setup();
        let path = dirs.bottle_logs_dir("demo").join("big.log");
        let line = "x".repeat(99) + "\n"; // 100 bytes
        let lines = (MAX_CHUNK / 100) as usize * 3;
        fs::write(&path, line.repeat(lines)).unwrap();

        // First read: only the tail, starting at a line boundary.
        let first = read(&dirs, Some("demo"), "big.log", None).unwrap();
        assert!(first.reset);
        assert!(first.text.len() as u64 <= MAX_CHUNK);
        assert!(first.text.starts_with('x') && first.text.ends_with('\n'));
        assert_eq!(first.offset, first.size);

        // Catching up from the start: whole lines, chunk by chunk.
        let mut offset = 0;
        let mut total = 0;
        loop {
            let chunk = read(&dirs, Some("demo"), "big.log", Some(offset)).unwrap();
            if chunk.text.is_empty() {
                break;
            }
            assert!(chunk.text.ends_with('\n'));
            total += chunk.text.len();
            offset = chunk.offset;
        }
        assert_eq!(total, lines * 100);
    }

    #[test]
    fn lists_newest_first_and_rejects_paths() {
        let (_tmp, dirs) = setup();
        let logs = dirs.bottle_logs_dir("demo");
        fs::write(logs.join("a.log"), "1").unwrap();
        fs::write(logs.join(".hidden"), "").unwrap();
        fs::create_dir(logs.join("sub")).unwrap();
        let names: Vec<_> = list(&dirs, Some("demo"))
            .unwrap()
            .into_iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(names, ["a.log"]);
        assert!(list(&dirs, None).unwrap().is_empty());
        assert!(list(&dirs, Some("../x")).is_err());

        for bad in ["../bottle.toml", "", ".hidden", "sub/x", r"..\x"] {
            assert!(read(&dirs, Some("demo"), bad, None).is_err(), "{bad:?}");
        }
    }
}
