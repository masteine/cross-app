//! `cross diag`: one archive with everything needed to understand a problem —
//! system check, settings, engine and bottle records, and recent logs.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use clap::Args;
use cross_bottle::launch::run_logs;
use cross_bottle::processes::running_wineservers;
use cross_core::config::{BottleStore, Config};
use cross_core::doctor;
use cross_core::logging::{timestamp_compact, timestamp_utc, LOG_FILE_PREFIX};
use cross_core::paths::AppDirs;
use cross_core::system::SystemInfo;

/// Largest slice of any single file copied into the archive (its end).
const MAX_FILE: u64 = 2 * 1024 * 1024;
/// Recent log files taken per directory and kind.
const RECENT: usize = 3;

#[derive(Args)]
pub struct DiagArgs {
    /// Where to write the archive (default: ./cross-diag-<time>.tar.gz)
    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

pub fn run(args: &DiagArgs, dirs: &AppDirs, config: &Config) -> Result<ExitCode> {
    let stamp = timestamp_compact(SystemTime::now());
    let name = format!("cross-diag-{stamp}");
    let output = match &args.output {
        Some(path) => path.clone(),
        None => std::env::current_dir()?.join(format!("{name}.tar.gz")),
    };

    let work = tempfile::tempdir().context("creating a temporary directory")?;
    let root = work.path().join(&name);
    let files = collect(dirs, config, &root)?;

    let status = Command::new("tar")
        .arg("-czf")
        .arg(&output)
        .arg("-C")
        .arg(work.path())
        .arg(&name)
        .status()
        .context("running tar")?;
    if !status.success() {
        bail!("tar failed with {status}");
    }
    let size = fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    tracing::info!(output = %output.display(), files, size, "diagnostics archive written");
    println!(
        "Wrote {} ({files} files, {} KB).",
        output.display(),
        size / 1024
    );
    println!(
        "It contains file paths (including your macOS user name), program names and logs; \
         no passwords are collected. Look through it before sharing."
    );
    Ok(ExitCode::SUCCESS)
}

/// Copy everything into `root`; returns the number of files written.
fn collect(dirs: &AppDirs, config: &Config, root: &Path) -> Result<usize> {
    fs::create_dir_all(root)?;
    let mut count = 0;
    let mut write = |rel: &str, text: &str| -> Result<()> {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        count += 1;
        Ok(())
    };

    let info = SystemInfo::detect(&config.gptk_dir(dirs));
    let report = doctor::evaluate(&info);
    write(
        "summary.txt",
        &format!(
            "cross-app {}\ngenerated: {}\ndata dir: {}\nargs: {:?}\n",
            env!("CARGO_PKG_VERSION"),
            timestamp_utc(SystemTime::now()),
            dirs.root().display(),
            std::env::args().collect::<Vec<_>>(),
        ),
    )?;
    write(
        "doctor.json",
        &serde_json::to_string_pretty(&serde_json::json!({ "system": info, "report": report }))?,
    )?;

    let mut copied = 0;
    let mut copy = |src: &Path, rel: &str| -> Result<()> {
        if src.is_file() {
            copy_tail(src, &root.join(rel), MAX_FILE)?;
            copied += 1;
        }
        Ok(())
    };

    for file in [
        "config.toml",
        "catalog.json",
        "verbs.toml",
        "steam-patches.toml",
    ] {
        copy(&dirs.root().join(file), &format!("settings/{file}"))?;
    }
    if let Ok(entries) = fs::read_dir(dirs.engines_dir()) {
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            copy(
                &entry.path().join("engine.toml"),
                &format!("engines/{id}.toml"),
            )?;
        }
    }
    for file in recent_cross_logs(&dirs.global_logs_dir()) {
        let name = file_name(&file);
        copy(&file, &format!("logs/{name}"))?;
    }

    let mut running = Vec::new();
    for bottle in BottleStore::new(dirs).list().unwrap_or_default() {
        let id = &bottle.id;
        copy(&dirs.bottle_file(id), &format!("bottles/{id}/bottle.toml"))?;
        let logs = dirs.bottle_logs_dir(id);
        let mut picked = recent_cross_logs(&logs);
        let runs = run_logs(&logs);
        picked.extend(runs.iter().rev().take(RECENT).cloned());
        picked.push(logs.join("steam.log"));
        for file in picked {
            let name = file_name(&file);
            copy(&file, &format!("bottles/{id}/logs/{name}"))?;
        }
        if running_wineservers(&dirs.bottle_prefix(id)).is_ok_and(|p| !p.is_empty()) {
            running.push(id.clone());
        }
    }
    write("running-bottles.txt", &format!("{}\n", running.join("\n")))?;
    Ok(count + copied)
}

/// The newest daily cross-app logs in `dir`.
fn recent_cross_logs(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| file_name(p).starts_with(LOG_FILE_PREFIX))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files.into_iter().rev().take(RECENT).collect()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Copy the last `max` bytes of `src` (logs grow; their end is what matters).
fn copy_tail(src: &Path, dst: &Path, max: u64) -> Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut input = File::open(src).with_context(|| format!("opening {}", src.display()))?;
    let len = input.metadata()?.len();
    let mut output = File::create(dst).with_context(|| format!("creating {}", dst.display()))?;
    if len > max {
        input.seek(SeekFrom::Start(len - max))?;
        writeln!(output, "[... first {} bytes omitted ...]", len - max)?;
    }
    let mut buf = Vec::new();
    input.take(max).read_to_end(&mut buf)?;
    output.write_all(&buf)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use cross_core::model::Bottle;

    use super::*;

    #[test]
    fn copy_tail_keeps_the_end_of_big_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("big.log");
        fs::write(&src, "0123456789").unwrap();
        let dst = tmp.path().join("out/big.log");
        copy_tail(&src, &dst, 4).unwrap();
        let text = fs::read_to_string(&dst).unwrap();
        assert!(
            text.starts_with("[... first 6 bytes omitted ...]"),
            "{text}"
        );
        assert!(text.ends_with("6789"), "{text}");

        copy_tail(&src, &dst, 100).unwrap();
        assert_eq!(fs::read_to_string(&dst).unwrap(), "0123456789");
    }

    #[test]
    fn collects_settings_records_and_recent_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        let config = Config::default();
        config.save(&dirs).unwrap();
        BottleStore::new(&dirs)
            .save(&Bottle::new("demo", "Demo").unwrap())
            .unwrap();
        let logs = dirs.bottle_logs_dir("demo");
        fs::create_dir_all(&logs).unwrap();
        for day in 10..16 {
            fs::write(logs.join(format!("cross.log.2026-10-{day}")), "x").unwrap();
        }
        fs::write(logs.join("run-20261014-120000.log"), "wine").unwrap();
        fs::create_dir_all(dirs.global_logs_dir()).unwrap();
        fs::write(dirs.global_logs_dir().join("cross.log.2026-10-14"), "g").unwrap();

        let root = tmp.path().join("diag");
        let count = collect(&dirs, &config, &root).unwrap();
        assert!(count >= 8, "{count}");
        for rel in [
            "summary.txt",
            "doctor.json",
            "settings/config.toml",
            "logs/cross.log.2026-10-14",
            "bottles/demo/bottle.toml",
            "bottles/demo/logs/run-20261014-120000.log",
            "bottles/demo/logs/cross.log.2026-10-15",
            "running-bottles.txt",
        ] {
            assert!(root.join(rel).is_file(), "missing {rel}");
        }
        // Only the newest daily logs.
        assert!(!root.join("bottles/demo/logs/cross.log.2026-10-12").exists());
        let doctor: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("doctor.json")).unwrap()).unwrap();
        assert!(doctor["report"]["checks"].is_array());
    }
}
