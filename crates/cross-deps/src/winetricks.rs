//! Fallback for verbs we do not carry: run the system `winetricks`, if installed.
//!
//! winetricks is a third-party shell script that downloads and runs installers.
//! It is only used when the user asks for a verb that is not in our catalog, and
//! the CLI says so before running it.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use cross_bottle::Runtime;

use crate::catalog::validate_verb_name;
use crate::{Error, Result};

/// Where Homebrew puts binaries on Apple Silicon and Intel Macs.
const EXTRA_DIRS: [&str; 2] = ["/opt/homebrew/bin", "/usr/local/bin"];

/// Locate `winetricks` in `PATH` and the usual Homebrew directories.
pub fn find() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(EXTRA_DIRS.iter().map(PathBuf::from));
    find_in(&dirs)
}

pub fn find_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|d| d.join("winetricks"))
        .find(|p| p.is_file())
}

/// Run `winetricks -q <verb>` against the bottle with its engine's Wine, showing
/// winetricks' own output on the terminal.
pub fn run(winetricks: &Path, runtime: &Runtime, verb: &str) -> Result<()> {
    // The name goes on another program's command line: no flags, no surprises.
    validate_verb_name(verb)?;
    tracing::info!(%verb, winetricks = %winetricks.display(), "running winetricks");
    let status = runtime
        .command_for(winetricks)
        .args(["-q", verb])
        .stdin(Stdio::null())
        .status()
        .map_err(|e| Error::Winetricks(format!("could not start {}: {e}", winetricks.display())))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Winetricks(format!(
            "`winetricks -q {verb}` exited with {status}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_winetricks_in_the_first_matching_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        assert_eq!(find_in(&[a.clone(), b.clone()]), None);
        std::fs::write(b.join("winetricks"), "").unwrap();
        assert_eq!(find_in(&[a.clone(), b.clone()]), Some(b.join("winetricks")));
        std::fs::write(a.join("winetricks"), "").unwrap();
        assert_eq!(find_in(&[a.clone(), b]), Some(a.join("winetricks")));
    }
}
