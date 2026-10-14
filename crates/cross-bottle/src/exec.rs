//! Running helper commands with a timeout and captured output.

use std::fs::{self, File};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::{Error, Result};

#[derive(Debug)]
pub struct Captured {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl Captured {
    /// reg.exe says `Unable to find the specified registry key/value`; depending on
    /// how Wine's console is wired that lands on either stream.
    pub fn reports_not_found(&self) -> bool {
        self.stdout.contains("Unable to find") || self.stderr.contains("Unable to find")
    }

    /// stderr if present, otherwise stdout; trimmed. For error messages.
    pub fn message(&self) -> String {
        let text = if self.stderr.trim().is_empty() {
            &self.stdout
        } else {
            &self.stderr
        };
        let text = text.trim();
        if text.is_empty() {
            format!("exited with {}", self.status)
        } else {
            text.to_owned()
        }
    }
}

/// Run `command` to completion, capturing its output, killing it after `timeout`.
///
/// Output goes to temp files rather than pipes: `wineboot` leaves a `wineserver`
/// behind that inherits its stdio, and a pipe would stay open (blocking us)
/// until that server exits.
pub fn run_captured(mut command: Command, what: &str, timeout: Duration) -> Result<Captured> {
    let dir = tempfile::tempdir().map_err(|e| Error::io(std::env::temp_dir(), e))?;
    let out_path = dir.path().join("stdout");
    let err_path = dir.path().join("stderr");
    let open = |p: &std::path::Path| File::create(p).map_err(|e| Error::io(p, e));

    command
        .stdin(Stdio::null())
        .stdout(open(&out_path)?)
        .stderr(open(&err_path)?);
    tracing::debug!(%what, ?command, "running");

    let mut child = command.spawn().map_err(|e| Error::Wine {
        what: what.to_owned(),
        message: format!("could not start {:?}: {e}", command.get_program()),
    })?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(|e| Error::io(dir.path(), e))? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Timeout {
                    what: what.to_owned(),
                    seconds: timeout.as_secs(),
                });
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    };

    let read = |p: &std::path::Path| fs::read(p).map(|b| String::from_utf8_lossy(&b).into_owned());
    let captured = Captured {
        status,
        stdout: read(&out_path).map_err(|e| Error::io(&out_path, e))?,
        stderr: read(&err_path).map_err(|e| Error::io(&err_path, e))?,
    };
    tracing::debug!(%what, status = %captured.status, "finished");
    Ok(captured)
}

/// Like [`run_captured`] but a non-zero exit becomes an error.
pub fn run_checked(command: Command, what: &str, timeout: Duration) -> Result<Captured> {
    let captured = run_captured(command, what, timeout)?;
    if captured.status.success() {
        Ok(captured)
    } else {
        Err(Error::Wine {
            what: what.to_owned(),
            message: captured.message(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut c = Command::new("/bin/sh");
        c.arg("-c").arg(script);
        c
    }

    #[test]
    fn captures_output_and_status() {
        let out = run_captured(
            sh("echo hi; echo oops >&2; exit 3"),
            "t",
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(out.stdout.trim(), "hi");
        assert_eq!(out.stderr.trim(), "oops");
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(out.message(), "oops");
    }

    #[test]
    fn checked_turns_failure_into_error() {
        let err =
            run_checked(sh("echo bad >&2; exit 1"), "thing", Duration::from_secs(5)).unwrap_err();
        assert!(err.to_string().contains("thing failed: bad"), "{err}");
    }

    #[test]
    fn times_out_and_kills() {
        let started = Instant::now();
        let err = run_captured(sh("sleep 30"), "slow", Duration::from_millis(200)).unwrap_err();
        assert!(matches!(err, Error::Timeout { .. }), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn lingering_background_child_does_not_block() {
        // Mimics wineboot leaving wineserver running with inherited stdio.
        let started = Instant::now();
        let out = run_captured(sh("sleep 5 & echo done"), "bg", Duration::from_secs(10)).unwrap();
        assert_eq!(out.stdout.trim(), "done");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn missing_program_is_reported() {
        let err = run_captured(
            Command::new("/nonexistent/wine"),
            "x",
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Wine { .. }), "{err}");
    }
}
