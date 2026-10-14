//! Turns a [`SystemInfo`] snapshot into a list of pass/warn/fail checks for `cross doctor`.

use std::fmt;

use serde::Serialize;

use crate::system::{MacVersion, SystemInfo};

/// Oldest macOS we consider usable at all.
const MIN_MACOS: MacVersion = MacVersion::new(14, 0, 0);
/// macOS the current Game Porting Toolkit targets.
const RECOMMENDED_MACOS: MacVersion = MacVersion::new(15, 0, 0);
/// First macOS where Rosetta 2 shrinks to a subset for legacy games.
const ROSETTA_LIMITED_FROM: u32 = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Fail,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Fail => "fail",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
}

impl Check {
    fn new(name: &'static str, status: Status, detail: impl Into<String>) -> Self {
        Self {
            name,
            status,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
}

impl Report {
    pub fn has_failures(&self) -> bool {
        self.checks.iter().any(|c| c.status == Status::Fail)
    }

    pub fn has_warnings(&self) -> bool {
        self.checks.iter().any(|c| c.status == Status::Warn)
    }
}

pub fn evaluate(info: &SystemInfo) -> Report {
    Report {
        checks: vec![
            check_arch(info),
            check_chip(info),
            check_macos(info),
            check_rosetta(info),
            check_xcode_clt(info),
            check_gptk(info),
        ],
    }
}

fn check_arch(info: &SystemInfo) -> Check {
    match info.arch.as_str() {
        "aarch64" => Check::new("architecture", Status::Ok, "arm64 (Apple Silicon)"),
        other => Check::new(
            "architecture",
            Status::Fail,
            format!("{other}: cross-app supports Apple Silicon only (Intel Macs are out of scope)"),
        ),
    }
}

fn check_chip(info: &SystemInfo) -> Check {
    match (&info.chip, &info.cpu_brand) {
        (Some(chip), _) => Check::new("chip", Status::Ok, chip.to_string()),
        (None, Some(brand)) => {
            Check::new("chip", Status::Warn, format!("unrecognised CPU: {brand}"))
        }
        (None, None) => Check::new("chip", Status::Warn, "could not read the CPU model"),
    }
}

fn check_macos(info: &SystemInfo) -> Check {
    match info.macos {
        None => Check::new("macOS", Status::Warn, "could not read the macOS version"),
        Some(v) if v < MIN_MACOS => Check::new(
            "macOS",
            Status::Fail,
            format!("{v}: macOS {} or newer is required", MIN_MACOS.major),
        ),
        Some(v) if v < RECOMMENDED_MACOS => Check::new(
            "macOS",
            Status::Warn,
            format!(
                "{v}: macOS {} or newer is recommended for the current Game Porting Toolkit",
                RECOMMENDED_MACOS.major
            ),
        ),
        Some(v) => Check::new("macOS", Status::Ok, v.to_string()),
    }
}

fn check_rosetta(info: &SystemInfo) -> Check {
    let limited = info.macos.is_some_and(|v| v.major >= ROSETTA_LIMITED_FROM);
    match (info.rosetta_installed, limited) {
        (_, true) => Check::new(
            "Rosetta 2",
            Status::Warn,
            format!(
                "macOS {ROSETTA_LIMITED_FROM}+ keeps only a legacy-games subset of Rosetta; \
                 use an engine that does not need it"
            ),
        ),
        (true, false) => Check::new("Rosetta 2", Status::Ok, "installed"),
        (false, false) => Check::new(
            "Rosetta 2",
            Status::Warn,
            "not installed; required by Intel Wine engines (install: softwareupdate --install-rosetta)",
        ),
    }
}

fn check_xcode_clt(info: &SystemInfo) -> Check {
    match &info.xcode_clt {
        Some(path) => Check::new(
            "Xcode Command Line Tools",
            Status::Ok,
            path.display().to_string(),
        ),
        None => Check::new(
            "Xcode Command Line Tools",
            Status::Warn,
            "not installed (install: xcode-select --install)",
        ),
    }
}

fn check_gptk(info: &SystemInfo) -> Check {
    match &info.gptk {
        Some(gptk) => {
            let version = gptk.version.as_deref().unwrap_or("unknown version");
            Check::new(
                "Game Porting Toolkit",
                Status::Ok,
                format!("{version} at {}", gptk.dir.display()),
            )
        }
        None => Check::new(
            "Game Porting Toolkit",
            Status::Warn,
            "not imported; DirectX 12 games need D3DMetal (see `cross engine import-gptk`)",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{AppleChip, GptkInfo};

    fn healthy() -> SystemInfo {
        SystemInfo {
            arch: "aarch64".into(),
            cpu_brand: Some("Apple M1 Pro".into()),
            chip: AppleChip::parse("Apple M1 Pro"),
            macos: Some(MacVersion::new(26, 5, 2)),
            rosetta_installed: true,
            xcode_clt: Some("/Library/Developer/CommandLineTools".into()),
            gptk: Some(GptkInfo {
                dir: "/gptk".into(),
                version: Some("3.0".into()),
            }),
        }
    }

    fn status_of(report: &Report, name: &str) -> Status {
        report
            .checks
            .iter()
            .find(|c| c.name == name)
            .unwrap()
            .status
    }

    #[test]
    fn healthy_system_has_no_findings() {
        let report = evaluate(&healthy());
        assert!(!report.has_failures());
        assert!(!report.has_warnings(), "{report:?}");
    }

    #[test]
    fn intel_is_a_failure() {
        let mut info = healthy();
        info.arch = "x86_64".into();
        let report = evaluate(&info);
        assert!(report.has_failures());
        assert_eq!(status_of(&report, "architecture"), Status::Fail);
    }

    #[test]
    fn macos_thresholds() {
        let mut info = healthy();
        info.macos = Some(MacVersion::new(13, 6, 0));
        assert_eq!(status_of(&evaluate(&info), "macOS"), Status::Fail);
        info.macos = Some(MacVersion::new(14, 4, 0));
        assert_eq!(status_of(&evaluate(&info), "macOS"), Status::Warn);
        info.macos = None;
        assert_eq!(status_of(&evaluate(&info), "macOS"), Status::Warn);
    }

    #[test]
    fn missing_rosetta_warns_before_macos_28() {
        let mut info = healthy();
        info.rosetta_installed = false;
        assert_eq!(status_of(&evaluate(&info), "Rosetta 2"), Status::Warn);
    }

    #[test]
    fn rosetta_is_flagged_as_limited_from_macos_28() {
        let mut info = healthy();
        info.macos = Some(MacVersion::new(28, 0, 0));
        let report = evaluate(&info);
        assert_eq!(status_of(&report, "Rosetta 2"), Status::Warn);
        assert!(report
            .checks
            .iter()
            .any(|c| c.detail.contains("legacy-games")));
    }

    #[test]
    fn missing_gptk_and_clt_only_warn() {
        let mut info = healthy();
        info.gptk = None;
        info.xcode_clt = None;
        let report = evaluate(&info);
        assert!(!report.has_failures());
        assert_eq!(status_of(&report, "Game Porting Toolkit"), Status::Warn);
        assert_eq!(status_of(&report, "Xcode Command Line Tools"), Status::Warn);
    }
}
