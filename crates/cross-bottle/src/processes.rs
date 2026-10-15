//! Finding and stopping the processes of a bottle.
//!
//! macOS `ps` shows `WINEPREFIX` only for the `wineserver` (Wine rewrites the
//! environment and command line of its client processes), so "is this bottle
//! running" is decided by its wineserver, and the Windows-side process list
//! comes from `tasklist.exe` run inside the prefix.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::exec::{run_captured, run_captured_with, OutputLogging};
use crate::runtime::Runtime;
use crate::Result;

const STOP_TIMEOUT: Duration = Duration::from_secs(60);
const TASKLIST_TIMEOUT: Duration = Duration::from_secs(30);

/// A Windows process inside a bottle, as reported by `tasklist.exe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WinProcess {
    pub name: String,
    /// Windows-side PID (not a macOS PID).
    pub pid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopMode {
    /// Ask programs to exit cleanly (`wineboot -e -s`).
    Graceful,
    /// Kill the wineserver and everything attached to it (`wineserver -k`).
    Force,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StopOutcome {
    pub was_running: bool,
    /// wineserver processes still alive afterwards (0 = fully stopped).
    pub remaining: usize,
}

/// macOS PIDs of the wineserver(s) serving `prefix`, from `ps -E` output of the
/// form `pid ppid etime command... ENV=value ...`.
pub fn parse_wineserver_pids(ps_output: &str, prefix: &Path) -> Vec<u32> {
    let needle = format!("WINEPREFIX={}", prefix.display());
    ps_output
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, rest) = line.split_once(char::is_whitespace)?;
            let pid: u32 = pid.parse().ok()?;
            let head = command_head(rest);
            let is_wineserver = head.trim_end().ends_with("wineserver")
                || head.contains("/wineserver ")
                || head.contains("wineserver -");
            (is_wineserver && has_env(rest, &needle)).then_some(pid)
        })
        .collect()
}

/// The part of a `ps -E` line before the environment block: everything up to
/// the first ` NAME=` token with an upper-case name.
fn command_head(rest: &str) -> &str {
    let bytes = rest.as_bytes();
    for (i, _) in rest.match_indices(' ') {
        let tail = &bytes[i + 1..];
        let name_len = tail
            .iter()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || **b == b'_')
            .count();
        if name_len > 0 && tail.get(name_len) == Some(&b'=') && tail[0].is_ascii_uppercase() {
            return &rest[..i];
        }
    }
    rest
}

/// `needle` (`WINEPREFIX=<path>`) occurs as a whole environment entry.
fn has_env(rest: &str, needle: &str) -> bool {
    rest.match_indices(needle).any(|(i, _)| {
        let starts = i == 0 || rest.as_bytes()[i - 1] == b' ';
        let end = i + needle.len();
        let ends = end == rest.len() || rest.as_bytes()[end] == b' ';
        starts && ends
    })
}

/// Parse `tasklist` output (CRLF, fixed-width table after a `====` ruler).
pub fn parse_tasklist(output: &str) -> Vec<WinProcess> {
    let mut lines = output.lines().map(|l| l.trim_end_matches('\r'));
    if !lines.any(|l| l.starts_with("====")) {
        return Vec::new();
    }
    lines
        .filter_map(|line| {
            // Image names can contain spaces; PID is the first purely numeric field after them.
            let fields: Vec<&str> = line.split_whitespace().collect();
            let at = fields.iter().position(|f| f.parse::<u32>().is_ok())?;
            if at == 0 {
                return None;
            }
            Some(WinProcess {
                name: fields[..at].join(" "),
                pid: fields[at].parse().ok()?,
            })
        })
        .collect()
}

fn ps_output() -> Result<String> {
    let mut cmd = Command::new("ps");
    cmd.args(["-axwwE", "-o", "pid=,ppid=,etime=,command="]);
    // `-E` prints the environment of every process of this user: never log it.
    let out = run_captured_with(cmd, "ps", Duration::from_secs(15), OutputLogging::Omit)?;
    if !out.status.success() {
        return Err(crate::Error::Wine {
            what: "ps".into(),
            message: out.message(),
        });
    }
    Ok(out.stdout)
}

/// macOS PIDs of the wineserver(s) running for `prefix` (empty when stopped).
/// Needs no engine, so it also works for bottles whose engine was removed.
pub fn running_wineservers(prefix: &Path) -> Result<Vec<u32>> {
    Ok(parse_wineserver_pids(&ps_output()?, prefix))
}

impl Runtime {
    /// macOS PIDs of this bottle's wineserver (empty when nothing is running).
    pub fn wineserver_pids(&self) -> Result<Vec<u32>> {
        running_wineservers(self.prefix())
    }

    pub fn is_running(&self) -> Result<bool> {
        Ok(!self.wineserver_pids()?.is_empty())
    }

    /// Windows processes in the bottle; empty when it is not running. Never
    /// starts a wineserver.
    pub fn processes(&self) -> Result<Vec<WinProcess>> {
        if !self.is_running()? {
            return Ok(Vec::new());
        }
        let mut cmd = self.wine_command();
        cmd.arg("tasklist");
        let out = run_captured(cmd, "tasklist", TASKLIST_TIMEOUT)?;
        Ok(parse_tasklist(&out.stdout)
            .into_iter()
            .filter(|p| !p.name.eq_ignore_ascii_case("tasklist.exe"))
            .collect())
    }

    pub fn stop(&self, mode: StopMode) -> Result<StopOutcome> {
        let before = self.wineserver_pids()?;
        if before.is_empty() {
            return Ok(StopOutcome {
                was_running: false,
                remaining: 0,
            });
        }
        tracing::info!(bottle = self.bottle_id(), ?mode, "stopping bottle");
        match mode {
            StopMode::Graceful => {
                let mut cmd = self.wine_command();
                cmd.args(["wineboot", "-e", "-s"]);
                run_captured(cmd, "wineboot -e -s", STOP_TIMEOUT)?;
                self.wait_until_stopped(Duration::from_secs(15))?;
            }
            StopMode::Force => {
                let mut cmd = self.wineserver_command();
                cmd.arg("-k");
                run_captured(cmd, "wineserver -k", STOP_TIMEOUT)?;
                if self.wait_until_stopped(Duration::from_secs(5))? > 0 {
                    for pid in self.wineserver_pids()? {
                        let mut kill = Command::new("kill");
                        kill.args(["-9", &pid.to_string()]);
                        run_captured(kill, "kill", Duration::from_secs(5))?;
                    }
                    self.wait_until_stopped(Duration::from_secs(5))?;
                }
            }
        }
        Ok(StopOutcome {
            was_running: true,
            remaining: self.wineserver_pids()?.len(),
        })
    }

    /// Poll until no wineserver remains or `timeout` passes; returns how many remain.
    fn wait_until_stopped(&self, timeout: Duration) -> Result<usize> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = self.wineserver_pids()?.len();
            if remaining == 0 || Instant::now() >= deadline {
                return Ok(remaining);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    const PREFIX: &str = "/Users/me/Library/Application Support/cross-app/bottles/demo/prefix";

    fn ps_line(pid: u32, command: &str, prefix: &str) -> String {
        format!("{pid:>6} 1 00:23 {command} USER=me WINEPREFIX={prefix} WINEARCH=win64 HOME=/Users/me\n")
    }

    #[test]
    fn finds_only_the_wineserver_of_this_prefix() {
        let server = "/x/Wine Devel.app/Contents/Resources/wine/lib/wine/../../bin/wineserver";
        let mut ps = String::new();
        ps += &ps_line(100, server, PREFIX);
        ps += &ps_line(
            101,
            server,
            "/Users/me/Library/Application Support/cross-app/bottles/other/prefix",
        );
        ps += &ps_line(102, server, &format!("{PREFIX}2"));
        ps += &ps_line(103, "/bin/zsh", PREFIX); // a shell that merely exports WINEPREFIX
        ps += "   104     1 00:22 C:\\windows\\system32\\services.exe \n";
        assert_eq!(
            parse_wineserver_pids(&ps, &PathBuf::from(PREFIX)),
            vec![100]
        );
    }

    #[test]
    fn prefix_must_match_a_whole_env_entry() {
        let server = "/e/bin/wineserver";
        let line = format!("7 1 00:01 {server} FOO=bar XWINEPREFIX={PREFIX} A=b\n");
        assert!(parse_wineserver_pids(&line, &PathBuf::from(PREFIX)).is_empty());
        let line = format!("7 1 00:01 {server} FOO=bar WINEPREFIX={PREFIX}\n");
        assert_eq!(
            parse_wineserver_pids(&line, &PathBuf::from(PREFIX)),
            vec![7]
        );
    }

    #[test]
    fn command_head_stops_at_the_environment() {
        assert_eq!(
            command_head("/a b/wineserver USER=x B=y"),
            "/a b/wineserver"
        );
        assert_eq!(
            command_head("/bin/foo --flag=1 USER=x"),
            "/bin/foo --flag=1"
        );
        assert_eq!(command_head("plain"), "plain");
    }

    #[test]
    fn parses_tasklist_table() {
        let out = "\r\nImage Name                     PID Session Name        Session#    Mem Usage\r\n\
                   ========================= ======== ================ =========== ============\r\n\
                   services.exe                    64 Console                    1          0 K\r\n\
                   my game.exe                    240 Console                    1          0 K\r\n\
                   tasklist.exe                   300 Console                    1          0 K\r\n";
        let procs = parse_tasklist(out);
        assert_eq!(
            procs,
            vec![
                WinProcess {
                    name: "services.exe".into(),
                    pid: 64
                },
                WinProcess {
                    name: "my game.exe".into(),
                    pid: 240
                },
                WinProcess {
                    name: "tasklist.exe".into(),
                    pid: 300
                },
            ]
        );
        assert!(parse_tasklist("no table here").is_empty());
    }
}
