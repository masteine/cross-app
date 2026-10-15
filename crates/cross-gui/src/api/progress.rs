//! `cross_engine::Progress` turned into events the page can render.

use std::time::{Duration, Instant};

use cross_engine::Progress;
use serde::Serialize;

/// Minimum gap between two download updates for the same step.
const MIN_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Stage {
    Downloading,
    CacheHit,
    Verifying,
    Extracting,
}

/// One progress update of a long task (`task://progress`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProgressEvent {
    pub task: String,
    /// What is being worked on: an engine, component, verb or `SteamSetup.exe`.
    pub label: String,
    pub stage: Stage,
    pub done: u64,
    pub total: Option<u64>,
    /// 0-100 when the size is known.
    pub percent: Option<u8>,
}

impl ProgressEvent {
    pub fn new(task: &str, progress: Progress<'_>) -> Self {
        let (label, stage, done, total) = match progress {
            Progress::Downloading { label, done, total } => {
                (label, Stage::Downloading, done, total)
            }
            Progress::CacheHit { label } => (label, Stage::CacheHit, 0, None),
            Progress::Verifying { label } => (label, Stage::Verifying, 0, None),
            Progress::Extracting { label } => (label, Stage::Extracting, 0, None),
        };
        let percent = total
            .filter(|t| *t > 0)
            .map(|t| u8::try_from(done.min(t) * 100 / t).unwrap_or(100));
        Self {
            task: task.to_owned(),
            label: label.to_owned(),
            stage,
            done,
            total,
            percent,
        }
    }
}

/// Drops download updates that would not change what the user sees, so a fast
/// download does not flood the IPC channel. Stage changes always pass.
#[derive(Debug, Default)]
pub struct Throttle {
    last: Option<(String, Stage, Option<u8>, Instant)>,
}

impl Throttle {
    pub fn admit(&mut self, event: &ProgressEvent, now: Instant) -> bool {
        let pass = match &self.last {
            None => true,
            Some((label, stage, percent, at)) => {
                *label != event.label
                    || *stage != event.stage
                    || (event.percent != *percent && now.duration_since(*at) >= MIN_INTERVAL)
                    || (event.percent.is_none() && now.duration_since(*at) >= MIN_INTERVAL)
            }
        };
        if pass {
            self.last = Some((event.label.clone(), event.stage.clone(), event.percent, now));
        }
        pass
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn downloading(done: u64, total: Option<u64>) -> ProgressEvent {
        ProgressEvent::new(
            "t1",
            Progress::Downloading {
                label: "wine",
                done,
                total,
            },
        )
    }

    #[test]
    fn maps_every_stage_with_percentages() {
        let e = downloading(50, Some(200));
        assert_eq!((e.stage.clone(), e.percent), (Stage::Downloading, Some(25)));
        assert_eq!(downloading(10, None).percent, None);
        assert_eq!(downloading(10, Some(0)).percent, None);
        // A server that sends more than it announced never exceeds 100 %.
        assert_eq!(downloading(300, Some(200)).percent, Some(100));

        let e = ProgressEvent::new("t1", Progress::CacheHit { label: "dxvk" });
        assert_eq!((e.label.as_str(), e.stage), ("dxvk", Stage::CacheHit));
        let e = ProgressEvent::new("t1", Progress::Extracting { label: "dxvk" });
        assert_eq!(e.stage, Stage::Extracting);
        let json =
            serde_json::to_value(ProgressEvent::new("t1", Progress::Verifying { label: "x" }))
                .unwrap();
        assert_eq!(json["stage"], "verifying");
        assert_eq!(json["task"], "t1");
    }

    #[test]
    fn throttle_keeps_stage_changes_and_drops_noise() {
        let mut throttle = Throttle::default();
        let t0 = Instant::now();
        assert!(throttle.admit(&downloading(1, Some(100)), t0));
        // Same percent, or too soon: dropped.
        assert!(!throttle.admit(&downloading(1, Some(100)), t0 + MIN_INTERVAL * 2));
        assert!(!throttle.admit(&downloading(2, Some(100)), t0));
        assert!(throttle.admit(&downloading(2, Some(100)), t0 + MIN_INTERVAL));
        // A new stage always gets through.
        let extracting = ProgressEvent::new("t1", Progress::Extracting { label: "wine" });
        assert!(throttle.admit(&extracting, t0 + MIN_INTERVAL));
        // Unknown size: time-based.
        assert!(throttle.admit(&downloading(5, None), t0 + MIN_INTERVAL));
        assert!(!throttle.admit(&downloading(6, None), t0 + MIN_INTERVAL));
        assert!(throttle.admit(&downloading(7, None), t0 + MIN_INTERVAL * 3));
    }
}
