//! `cross crash ...`: local crash reports written by the panic hook in
//! `cross_core::crash`. Nothing is ever uploaded; `show` prints a report so the
//! user can attach it to a bug report by hand.

use std::process::ExitCode;

use anyhow::Result;
use clap::Subcommand;
use cross_core::crash::{self, format_utc};
use cross_core::paths::AppDirs;

use crate::prompt::confirm;

#[derive(Subcommand)]
pub enum CrashCommand {
    /// List saved crash reports, newest first
    List {
        #[arg(long)]
        json: bool,
    },
    /// Print a crash report (default: the newest)
    Show {
        /// Report id from `cross crash list`, or `latest`
        #[arg(default_value = "latest")]
        id: String,
    },
    /// Delete all crash reports
    Clear {
        /// Do not ask for confirmation
        #[arg(short, long)]
        yes: bool,
    },
}

pub fn run(command: CrashCommand, dirs: &AppDirs) -> Result<ExitCode> {
    match command {
        CrashCommand::List { json } => {
            let reports = crash::list(dirs)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&reports)?);
            } else if reports.is_empty() {
                println!("No crash reports.");
            } else {
                let width = reports.iter().map(|r| r.id.len()).max().unwrap_or(2);
                println!("{:width$}  {:20}  MESSAGE", "ID", "TIME (UTC)");
                for r in &reports {
                    println!(
                        "{:width$}  {:20}  {}",
                        r.id,
                        format_utc(r.created_unix),
                        r.message.as_deref().unwrap_or("?")
                    );
                }
                println!(
                    "\nReports stay in {} and are never uploaded.",
                    crash::crashes_dir(dirs).display()
                );
            }
        }
        CrashCommand::Show { id } => {
            let (_, text) = crash::read(dirs, &id)?;
            print!("{text}");
        }
        CrashCommand::Clear { yes } => {
            let count = crash::list(dirs)?.len();
            if count == 0 {
                println!("No crash reports.");
                return Ok(ExitCode::SUCCESS);
            }
            if !confirm(&format!("Delete {count} crash report(s)?"), yes)? {
                println!("Cancelled.");
                return Ok(ExitCode::from(1));
            }
            let removed = crash::clear(dirs)?;
            println!("Deleted {removed} crash report(s).");
        }
    }
    Ok(ExitCode::SUCCESS)
}
