//! The only data telemetry ever records, and how it is made anonymous.
//!
//! An event holds coarse facts about the Mac and the engine plus whether the
//! launch worked. It never holds paths, program or game names, bottle ids,
//! user names, host names, timestamps, IP-derived data or any identifier that
//! links two events to the same machine.

use cross_core::model::GraphicsBackend;
use cross_core::system::SystemInfo;
use cross_engine::catalog::Catalog;
use serde::{Deserialize, Serialize};

/// Bumped whenever a field is added, removed or changes meaning.
pub const SCHEMA_VERSION: u32 = 1;

/// Engine id reported for engines that are not in the built-in catalog: a
/// local `catalog.json` may use arbitrary, possibly identifying, ids.
pub const CUSTOM_ENGINE: &str = "custom";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A Windows program was started through `cross run` (or the GUI equivalent).
    Launch,
}

/// How a launch ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The program started and exited with code 0.
    Success,
    /// The program started but exited with a non-zero code or a signal.
    ExitedWithError,
    /// Wine could not start the program at all.
    FailedToStart,
}

/// One anonymous record. Field order is the order `cross telemetry show` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub schema: u32,
    pub kind: EventKind,
    /// cross-app version, e.g. `0.0.1`.
    pub app_version: String,
    /// macOS `major.minor` only (e.g. `26.5`): the patch level adds little and narrows the crowd.
    pub macos: Option<String>,
    /// Parsed chip family, e.g. `Apple M1 Pro`; never the raw sysctl brand string.
    pub chip: Option<String>,
    /// Built-in catalog engine id, or [`CUSTOM_ENGINE`].
    pub engine: Option<String>,
    pub backend: Option<GraphicsBackend>,
    pub outcome: Outcome,
}

impl Event {
    /// Build a launch event from a system probe. Everything is reduced to the
    /// coarse values documented on [`Event`].
    pub fn launch(
        system: &SystemInfo,
        engine: Option<&str>,
        backend: Option<GraphicsBackend>,
        outcome: Outcome,
    ) -> Self {
        Self {
            schema: SCHEMA_VERSION,
            kind: EventKind::Launch,
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            macos: system.macos.map(|v| format!("{}.{}", v.major, v.minor)),
            chip: system.chip.as_ref().map(ToString::to_string),
            engine: engine.map(anonymize_engine),
            backend,
            outcome,
        }
    }
}

/// Keep ids from the catalog compiled into cross-app; anything else becomes [`CUSTOM_ENGINE`].
fn anonymize_engine(id: &str) -> String {
    if Catalog::builtin().engine(id).is_some() {
        id.to_owned()
    } else {
        CUSTOM_ENGINE.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cross_core::system::{AppleChip, MacVersion};

    fn system() -> SystemInfo {
        SystemInfo {
            arch: "aarch64".into(),
            cpu_brand: Some("Apple M1 Pro (serial-like detail)".into()),
            chip: AppleChip::parse("Apple M1 Pro"),
            macos: Some(MacVersion::new(26, 5, 2)),
            rosetta_installed: true,
            xcode_clt: Some("/Users/someone/xcode".into()),
            gptk: None,
        }
    }

    #[test]
    fn launch_event_is_coarse() {
        let builtin = Catalog::builtin();
        let known = &builtin.engines[0].id;
        let event = Event::launch(
            &system(),
            Some(known),
            Some(GraphicsBackend::Dxvk),
            Outcome::Success,
        );
        assert_eq!(event.schema, SCHEMA_VERSION);
        assert_eq!(event.macos.as_deref(), Some("26.5"));
        assert_eq!(event.chip.as_deref(), Some("Apple M1 Pro"));
        assert_eq!(event.engine.as_deref(), Some(known.as_str()));

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""kind":"launch""#), "{json}");
        assert!(json.contains(r#""backend":"dxvk""#), "{json}");
        assert!(json.contains(r#""outcome":"success""#), "{json}");
        // Nothing from the raw probe that could identify the machine or user.
        assert!(!json.contains("serial"), "{json}");
        assert!(!json.contains("/Users"), "{json}");
        assert!(!json.contains("26.5.2"), "{json}");
    }

    #[test]
    fn unknown_engines_are_reported_as_custom() {
        let event = Event::launch(
            &system(),
            Some("my-secret-build-for-alice"),
            None,
            Outcome::FailedToStart,
        );
        assert_eq!(event.engine.as_deref(), Some(CUSTOM_ENGINE));
    }

    #[test]
    fn missing_probe_values_stay_empty() {
        let mut info = system();
        info.chip = None;
        info.macos = None;
        let event = Event::launch(&info, None, None, Outcome::ExitedWithError);
        assert_eq!((event.chip, event.macos, event.engine), (None, None, None));
    }

    #[test]
    fn events_round_trip_as_json() {
        let event = Event::launch(
            &system(),
            None,
            Some(GraphicsBackend::D3dMetal),
            Outcome::Success,
        );
        let back: Event = serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(back, event);
    }
}
