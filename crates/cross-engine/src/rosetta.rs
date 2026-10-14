//! Installing Rosetta 2.
//!
//! Installation is an explicit, user-confirmed action (the CLI asks first);
//! this module only knows how to run it.

use std::process::Command;

use crate::{Error, Result};

const PROGRAM: &str = "softwareupdate";
const ARGS: [&str; 2] = ["--install-rosetta", "--agree-to-license"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    AlreadyInstalled,
    Installed,
}

/// The exact command line used, for showing to the user before running it.
pub fn command_line() -> String {
    format!("{PROGRAM} {}", ARGS.join(" "))
}

/// Install Rosetta 2 unless it is already present. Output is passed through to
/// the terminal because `softwareupdate` can ask for an administrator password.
pub fn install() -> Result<Outcome> {
    if cross_core::system::rosetta_installed() {
        return Ok(Outcome::AlreadyInstalled);
    }
    tracing::info!(command = %command_line(), "installing Rosetta 2");
    let status = Command::new(PROGRAM)
        .args(ARGS)
        .status()
        .map_err(|e| Error::Command {
            program: PROGRAM.to_owned(),
            message: e.to_string(),
        })?;
    if status.success() {
        Ok(Outcome::Installed)
    } else {
        Err(Error::Command {
            program: PROGRAM.to_owned(),
            message: format!(
                "exited with {status}; administrator rights may be required: sudo {}",
                command_line()
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_matches_apples_documented_invocation() {
        assert_eq!(
            command_line(),
            "softwareupdate --install-rosetta --agree-to-license"
        );
    }
}
