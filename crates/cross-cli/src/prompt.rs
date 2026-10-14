//! Interactive yes/no confirmation shared by destructive or system-changing commands.

use std::io::{IsTerminal, Write};

use anyhow::{bail, Context, Result};

/// Ask `question`; `assume_yes` skips the prompt. Without a terminal and without
/// `--yes` the command refuses instead of guessing.
pub fn confirm(question: &str, assume_yes: bool) -> Result<bool> {
    if assume_yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        bail!("refusing to continue without confirmation; pass --yes");
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("could not read the answer")?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
