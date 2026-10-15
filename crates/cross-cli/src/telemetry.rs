//! `cross telemetry ...`: opt-in, anonymous launch statistics (local queue only).

use std::process::ExitCode;

use anyhow::Result;
use clap::Subcommand;
use cross_core::paths::AppDirs;
use cross_telemetry::store::{DO_NOT_TRACK_ENV, MAX_QUEUED_EVENTS};
use cross_telemetry::Telemetry;

#[derive(Subcommand)]
pub enum TelemetryCommand {
    /// Show whether telemetry is on, how many events are queued and where the files are
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Opt in: record anonymous launch results locally (nothing is sent anywhere yet)
    Enable,
    /// Opt out and delete every queued event
    Disable,
    /// Print the queued events exactly as they would be sent, one JSON object per line
    Show,
}

/// What a report contains, printed on `enable` and `status` so consent is informed.
const COLLECTED: &str = "\
Each launch report contains only: cross-app version, macOS major.minor (e.g. 26.5),
chip family (e.g. Apple M1 Pro), engine id from the built-in catalog (others become
\"custom\"), graphics backend, and whether the launch succeeded. No ids, paths, program
or game names, user or host names, or timestamps.";

pub fn run(command: TelemetryCommand, dirs: &AppDirs) -> Result<ExitCode> {
    let mut telemetry = Telemetry::load(dirs)?;
    match command {
        TelemetryCommand::Status { json } => status(&telemetry, json)?,
        TelemetryCommand::Enable => {
            telemetry.enable()?;
            println!("Telemetry enabled.\n\n{COLLECTED}");
            println!(
                "\nReports are queued in {} (at most {MAX_QUEUED_EVENTS}).",
                telemetry.queue_path().display()
            );
            println!("No upload endpoint exists yet: nothing leaves this Mac.");
            if Telemetry::suppressed_by_env() {
                println!("note: {DO_NOT_TRACK_ENV} is set, so nothing is recorded while it is.");
            }
        }
        TelemetryCommand::Disable => {
            let removed = telemetry.disable()?;
            println!("Telemetry disabled; deleted {removed} queued event(s).");
        }
        TelemetryCommand::Show => {
            for event in telemetry.queued()? {
                println!("{}", serde_json::to_string(&event)?);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn status(telemetry: &Telemetry, json: bool) -> Result<()> {
    let settings = telemetry.settings();
    let queued = telemetry.queued()?.len();
    let suppressed = Telemetry::suppressed_by_env();
    if json {
        let output = serde_json::json!({
            "enabled": settings.enabled,
            "decided": settings.decided,
            "recording": telemetry.is_recording(),
            "do_not_track": suppressed,
            "queued": queued,
            "settings_file": telemetry.settings_path(),
            "queue_file": telemetry.queue_path(),
            "endpoint": null,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    let state = match (settings.enabled, suppressed) {
        (true, false) => "enabled",
        (true, true) => "enabled, but suppressed by DO_NOT_TRACK",
        (false, _) if settings.decided => "disabled",
        (false, _) => "disabled (never enabled)",
    };
    println!("telemetry: {state}");
    println!(
        "queued:    {queued} event(s) in {}",
        telemetry.queue_path().display()
    );
    println!("settings:  {}", telemetry.settings_path().display());
    println!("endpoint:  none (nothing is ever sent)\n");
    println!("{COLLECTED}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::{CommandFactory, Parser};

    #[derive(Parser)]
    struct Wrapper {
        #[command(subcommand)]
        command: TelemetryCommand,
    }

    #[test]
    fn subcommands_parse() {
        Wrapper::command().debug_assert();
        for args in [
            &["x", "status"][..],
            &["x", "status", "--json"],
            &["x", "enable"],
            &["x", "disable"],
            &["x", "show"],
        ] {
            assert!(Wrapper::try_parse_from(args).is_ok(), "{args:?}");
        }
        assert!(Wrapper::try_parse_from(["x", "upload"]).is_err());
    }

    #[test]
    fn enable_show_disable_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        run(TelemetryCommand::Status { json: true }, &dirs).unwrap();
        run(TelemetryCommand::Enable, &dirs).unwrap();
        assert!(Telemetry::load(&dirs).unwrap().settings().enabled);
        run(TelemetryCommand::Show, &dirs).unwrap();
        run(TelemetryCommand::Disable, &dirs).unwrap();
        assert!(!Telemetry::load(&dirs).unwrap().settings().enabled);
    }
}
