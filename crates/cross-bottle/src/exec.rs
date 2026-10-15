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
pub fn run_captured(command: Command, what: &str, timeout: Duration) -> Result<Captured> {
    run_captured_with(command, what, timeout, OutputLogging::Tail)
}

/// Whether a command's output may be copied into the debug log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputLogging {
    /// The tail of stdout/stderr goes into the debug log.
    Tail,
    /// Only status and timing are logged: the output can hold secrets
    /// (e.g. `ps -E` prints every process's environment).
    Omit,
}

/// [`run_captured`] with control over what reaches the log.
pub fn run_captured_with(
    mut command: Command,
    what: &str,
    timeout: Duration,
    logging: OutputLogging,
) -> Result<Captured> {
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
    let started = Instant::now();
    let deadline = started + timeout;
    let status = loop {
        match child.try_wait().map_err(|e| Error::io(dir.path(), e))? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                tracing::warn!(%what, seconds = timeout.as_secs(), "timed out, killing");
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
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match logging {
        // Keep the tail of the output in the log file: it is what explains a failure later.
        OutputLogging::Tail => tracing::debug!(
            %what,
            status = %captured.status,
            elapsed_ms,
            stdout = %redact(tail(&captured.stdout, LOGGED_OUTPUT)),
            stderr = %redact(tail(&captured.stderr, LOGGED_OUTPUT)),
            "finished"
        ),
        OutputLogging::Omit => {
            tracing::debug!(%what, status = %captured.status, elapsed_ms, "finished (output not logged)");
        }
    }
    Ok(captured)
}

/// Bytes of a command's stdout/stderr copied into the debug log.
const LOGGED_OUTPUT: usize = 4096;

/// The last `max` bytes of `text`, cut at a character boundary, trimmed.
pub fn tail(text: &str, max: usize) -> &str {
    let text = text.trim();
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// Parts of variable names that suggest a secret value.
const SECRET_HINTS: [&str; 7] = [
    "TOKEN", "SECRET", "PASSWORD", "PASSWD", "KEY", "AUTH", "COOKIE",
];

/// Whether an environment variable name looks like it holds a secret.
pub fn is_secret_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    SECRET_HINTS.iter().any(|hint| upper.contains(hint))
}

/// Mask the values of `NAME=value` words whose name looks secret, so logs can be
/// shared (`cross diag`) without leaking tokens that ended up in some output.
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(eq) = rest.find('=') {
        let (before, after) = rest.split_at(eq);
        let name_start = before
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map_or(0, |i| i + 1);
        let name = &before[name_start..];
        out.push_str(before);
        out.push('=');
        let after = &after[1..];
        if !name.is_empty() && is_secret_name(name) {
            // A quoted value runs to its closing quote; a bare one ends at whitespace
            // or at the quoting/punctuation of Debug output (`["A=b", ...]`).
            let end = match after.chars().next() {
                Some(q @ ('"' | '\'')) => after[1..].find(q).map_or(after.len(), |i| i + 2),
                _ => after
                    .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ',' | ';'))
                    .unwrap_or(after.len()),
            };
            out.push_str("<redacted>");
            rest = &after[end..];
        } else {
            rest = after;
        }
    }
    out.push_str(rest);
    out
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
    fn tail_keeps_the_end_on_a_char_boundary() {
        assert_eq!(tail("  short \n", 100), "short");
        assert_eq!(tail("abcdef", 3), "def");
        // "é" is two bytes; never split it.
        assert_eq!(tail("aéb", 2), "b");
        assert_eq!(tail("aéb", 3), "éb");
    }

    #[test]
    fn secrets_are_redacted() {
        let text = "PATH=/bin API_TOKEN=abc123 HOME=/Users/me db_password=hunter2 x=1";
        assert_eq!(
            redact(text),
            "PATH=/bin API_TOKEN=<redacted> HOME=/Users/me db_password=<redacted> x=1"
        );
        assert_eq!(redact("no assignments here"), "no assignments here");
        assert_eq!(redact("SSH_AUTH_SOCK=/tmp/s"), "SSH_AUTH_SOCK=<redacted>");
        // Debug-formatted argument lists keep their quotes and commas.
        assert_eq!(
            redact(r#"["--env", "MY_TOKEN=abc", "--env", "FOO=bar"]"#),
            r#"["--env", "MY_TOKEN=<redacted>", "--env", "FOO=bar"]"#
        );
        assert_eq!(redact(r#"MY_TOKEN="a b c" X=1"#), "MY_TOKEN=<redacted> X=1");
        assert_eq!(redact("MY_TOKEN='abc"), "MY_TOKEN=<redacted>");
        assert!(is_secret_name("GITHUB_TOKEN"));
        assert!(!is_secret_name("WINEPREFIX"));
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
