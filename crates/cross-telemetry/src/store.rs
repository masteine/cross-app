//! The local consent setting (`telemetry.toml`) and the local event queue
//! (`telemetry/queue.jsonl`), both under the cross-app data directory.

use std::fs;
use std::path::{Path, PathBuf};

use cross_core::model::GraphicsBackend;
use cross_core::paths::AppDirs;
use cross_core::system::SystemInfo;
use serde::{Deserialize, Serialize};

use crate::event::{Event, Outcome};
use crate::{Error, Result};

/// Oldest events are dropped beyond this many, so the queue cannot grow without bound.
pub const MAX_QUEUED_EVENTS: usize = 500;

/// The informal `DO_NOT_TRACK` convention: any value other than empty or `0`
/// stops recording even when telemetry is enabled.
pub const DO_NOT_TRACK_ENV: &str = "DO_NOT_TRACK";

/// Consent state. A missing file means "never asked, disabled".
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Off unless the user explicitly runs `cross telemetry enable` (or opts in from the GUI).
    #[serde(default)]
    pub enabled: bool,
    /// The user has made a choice either way; the GUI asks at most once while this is false.
    #[serde(default)]
    pub decided: bool,
}

/// Telemetry settings plus the queue, bound to one data directory.
#[derive(Debug, Clone)]
pub struct Telemetry {
    settings_file: PathBuf,
    queue_file: PathBuf,
    settings: Settings,
}

impl Telemetry {
    /// `<data dir>/telemetry.toml`.
    pub fn settings_file(dirs: &AppDirs) -> PathBuf {
        dirs.root().join("telemetry.toml")
    }

    /// `<data dir>/telemetry/queue.jsonl`: one JSON event per line.
    pub fn queue_file(dirs: &AppDirs) -> PathBuf {
        dirs.root().join("telemetry").join("queue.jsonl")
    }

    pub fn load(dirs: &AppDirs) -> Result<Self> {
        let settings_file = Self::settings_file(dirs);
        let settings = match fs::read_to_string(&settings_file) {
            Ok(text) => toml::from_str(&text).map_err(|source| Error::TomlParse {
                path: settings_file.clone(),
                source,
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
            Err(e) => return Err(Error::io(&settings_file, e)),
        };
        Ok(Self {
            settings_file,
            queue_file: Self::queue_file(dirs),
            settings,
        })
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn settings_path(&self) -> &Path {
        &self.settings_file
    }

    pub fn queue_path(&self) -> &Path {
        &self.queue_file
    }

    /// Opt in.
    pub fn enable(&mut self) -> Result<()> {
        self.settings = Settings {
            enabled: true,
            decided: true,
        };
        self.save()?;
        tracing::debug!("telemetry enabled");
        Ok(())
    }

    /// Opt out and delete every queued event. Returns how many were deleted.
    pub fn disable(&mut self) -> Result<usize> {
        self.settings = Settings {
            enabled: false,
            decided: true,
        };
        self.save()?;
        let removed = self.clear_queue()?;
        tracing::debug!(removed, "telemetry disabled");
        Ok(removed)
    }

    /// `DO_NOT_TRACK` is set in the environment.
    pub fn suppressed_by_env() -> bool {
        std::env::var_os(DO_NOT_TRACK_ENV).is_some_and(|v| !v.is_empty() && v != "0")
    }

    /// Whether [`Telemetry::record`] would store anything right now.
    pub fn is_recording(&self) -> bool {
        self.settings.enabled && !Self::suppressed_by_env()
    }

    /// Append an event to the local queue if the user opted in. Returns whether it was stored.
    pub fn record(&self, event: &Event) -> Result<bool> {
        if !self.is_recording() {
            return Ok(false);
        }
        self.push(event)?;
        Ok(true)
    }

    fn push(&self, event: &Event) -> Result<()> {
        let mut events = self.queued()?;
        events.push(event.clone());
        let overflow = events.len().saturating_sub(MAX_QUEUED_EVENTS);
        if overflow > 0 {
            tracing::debug!(dropped = overflow, "telemetry queue full; dropping oldest");
        }
        self.write_queue(&events[overflow..])
    }

    /// Queued events, oldest first. Malformed lines (e.g. from a newer schema) are skipped.
    pub fn queued(&self) -> Result<Vec<Event>> {
        let text = match fs::read_to_string(&self.queue_file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(Error::io(&self.queue_file, e)),
        };
        Ok(text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| match serde_json::from_str(line) {
                Ok(event) => Some(event),
                Err(err) => {
                    tracing::warn!(%err, "skipping unreadable telemetry event");
                    None
                }
            })
            .collect())
    }

    /// Delete the queue file. Returns how many events it held.
    pub fn clear_queue(&self) -> Result<usize> {
        let count = self.queued()?.len();
        match fs::remove_file(&self.queue_file) {
            Ok(()) => Ok(count),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(Error::io(&self.queue_file, e)),
        }
    }

    fn save(&self) -> Result<()> {
        let text = toml::to_string_pretty(&self.settings)?;
        write_atomic(&self.settings_file, &text)
    }

    fn write_queue(&self, events: &[Event]) -> Result<()> {
        let mut text = String::new();
        for event in events {
            text.push_str(&serde_json::to_string(event)?);
            text.push('\n');
        }
        write_atomic(&self.queue_file, &text)
    }
}

/// Record a launch if, and only if, the user opted in.
///
/// Never fails and never blocks a launch: problems are logged and swallowed.
/// The system probe (`sysctl`, `sw_vers`) only runs when recording is on.
pub fn record_launch(
    dirs: &AppDirs,
    engine: Option<&str>,
    backend: Option<GraphicsBackend>,
    outcome: Outcome,
) -> bool {
    let telemetry = match Telemetry::load(dirs) {
        Ok(t) if t.is_recording() => t,
        Ok(_) => return false,
        Err(err) => {
            tracing::warn!(%err, "telemetry settings unreadable; not recording");
            return false;
        }
    };
    let system = SystemInfo::detect(&dirs.gptk_dir());
    let event = Event::launch(&system, engine, backend, outcome);
    match telemetry.record(&event) {
        Ok(stored) => stored,
        Err(err) => {
            tracing::warn!(%err, "could not queue telemetry event");
            false
        }
    }
}

/// Write via a temp file + rename so a crash never leaves a half-written file.
fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    fs::write(&tmp, text).map_err(|e| Error::io(&tmp, e))?;
    fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventKind, SCHEMA_VERSION};

    fn event(outcome: Outcome) -> Event {
        Event {
            schema: SCHEMA_VERSION,
            kind: EventKind::Launch,
            app_version: "0.0.1".into(),
            macos: Some("26.5".into()),
            chip: Some("Apple M1".into()),
            engine: Some("custom".into()),
            backend: Some(GraphicsBackend::Dxvk),
            outcome,
        }
    }

    fn setup() -> (tempfile::TempDir, AppDirs) {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        (tmp, dirs)
    }

    #[test]
    fn disabled_by_default_and_records_nothing() {
        let (_tmp, dirs) = setup();
        let telemetry = Telemetry::load(&dirs).unwrap();
        assert_eq!(telemetry.settings(), &Settings::default());
        assert!(!telemetry.is_recording());
        assert!(!telemetry.record(&event(Outcome::Success)).unwrap());
        assert!(!Telemetry::queue_file(&dirs).exists());
        assert!(!Telemetry::settings_file(&dirs).exists());
        assert!(!record_launch(&dirs, None, None, Outcome::Success));
        assert!(!Telemetry::queue_file(&dirs).exists());
    }

    #[test]
    fn enable_records_and_disable_deletes_queue() {
        let (_tmp, dirs) = setup();
        let mut telemetry = Telemetry::load(&dirs).unwrap();
        telemetry.enable().unwrap();

        let reloaded = Telemetry::load(&dirs).unwrap();
        assert_eq!(
            reloaded.settings(),
            &Settings {
                enabled: true,
                decided: true
            }
        );
        // DO_NOT_TRACK is not set in the test environment; guard anyway.
        if Telemetry::suppressed_by_env() {
            return;
        }
        assert!(reloaded.record(&event(Outcome::Success)).unwrap());
        assert!(reloaded.record(&event(Outcome::FailedToStart)).unwrap());
        let queued = reloaded.queued().unwrap();
        assert_eq!(queued.len(), 2);
        assert_eq!(queued[1].outcome, Outcome::FailedToStart);

        let text = fs::read_to_string(Telemetry::queue_file(&dirs)).unwrap();
        assert_eq!(text.lines().count(), 2, "{text}");

        assert_eq!(telemetry.disable().unwrap(), 2);
        assert!(!Telemetry::queue_file(&dirs).exists());
        let reloaded = Telemetry::load(&dirs).unwrap();
        assert!(!reloaded.settings().enabled);
        assert!(reloaded.settings().decided);
        assert!(!reloaded.record(&event(Outcome::Success)).unwrap());
    }

    #[test]
    fn record_launch_probes_the_system_when_enabled() {
        let (_tmp, dirs) = setup();
        Telemetry::load(&dirs).unwrap().enable().unwrap();
        if Telemetry::suppressed_by_env() {
            return;
        }
        assert!(record_launch(
            &dirs,
            Some("not-in-catalog"),
            Some(GraphicsBackend::D3dMetal),
            Outcome::ExitedWithError
        ));
        let queued = Telemetry::load(&dirs).unwrap().queued().unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(
            queued[0].engine.as_deref(),
            Some(crate::event::CUSTOM_ENGINE)
        );
        assert_eq!(queued[0].outcome, Outcome::ExitedWithError);
    }

    #[test]
    fn queue_is_capped_dropping_oldest() {
        let (_tmp, dirs) = setup();
        let mut telemetry = Telemetry::load(&dirs).unwrap();
        telemetry.enable().unwrap();
        if Telemetry::suppressed_by_env() {
            return;
        }
        let mut events: Vec<Event> = (0..MAX_QUEUED_EVENTS)
            .map(|_| event(Outcome::Success))
            .collect();
        events[0].outcome = Outcome::ExitedWithError;
        telemetry.write_queue(&events).unwrap();

        telemetry.record(&event(Outcome::FailedToStart)).unwrap();
        let queued = telemetry.queued().unwrap();
        assert_eq!(queued.len(), MAX_QUEUED_EVENTS);
        assert_eq!(queued[0].outcome, Outcome::Success, "oldest was dropped");
        assert_eq!(queued.last().unwrap().outcome, Outcome::FailedToStart);
    }

    #[test]
    fn unreadable_queue_lines_are_skipped() {
        let (_tmp, dirs) = setup();
        let telemetry = Telemetry::load(&dirs).unwrap();
        let line = serde_json::to_string(&event(Outcome::Success)).unwrap();
        fs::create_dir_all(Telemetry::queue_file(&dirs).parent().unwrap()).unwrap();
        fs::write(
            Telemetry::queue_file(&dirs),
            format!("{line}\n{{not json\n\n{line}\n"),
        )
        .unwrap();
        assert_eq!(telemetry.queued().unwrap().len(), 2);
        assert_eq!(telemetry.clear_queue().unwrap(), 2);
        assert_eq!(telemetry.clear_queue().unwrap(), 0);
    }

    #[test]
    fn malformed_settings_report_path() {
        let (_tmp, dirs) = setup();
        fs::write(Telemetry::settings_file(&dirs), "enabled = [").unwrap();
        let err = Telemetry::load(&dirs).unwrap_err();
        assert!(err.to_string().contains("telemetry.toml"), "{err}");
        // A broken settings file must never turn recording on.
        assert!(!record_launch(&dirs, None, None, Outcome::Success));
    }
}
