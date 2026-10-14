//! Detection of the host environment: chip, macOS version, Rosetta, Xcode CLT, GPTK.
//!
//! Probing (`SystemInfo::detect`) is separated from interpretation
//! (`crate::doctor::evaluate`) so the latter is testable with synthetic data.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

/// Marker file installed by `softwareupdate --install-rosetta`.
const ROSETTA_RUNTIME: &str = "/Library/Apple/usr/libexec/oah/libRosettaRuntime";

/// Whether Rosetta 2 is installed on this Mac.
pub fn rosetta_installed() -> bool {
    Path::new(ROSETTA_RUNTIME).exists()
}

/// An Apple Silicon chip such as "Apple M1 Pro".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppleChip {
    /// 1 for M1, 2 for M2, ...
    pub generation: u8,
    /// "Pro", "Max", "Ultra"; `None` for the base chip.
    pub variant: Option<String>,
}

impl AppleChip {
    /// Parse a `machdep.cpu.brand_string` value like `Apple M3 Max`.
    pub fn parse(brand: &str) -> Option<Self> {
        let mut words = brand.split_whitespace();
        if words.next()? != "Apple" {
            return None;
        }
        let model = words.next()?.strip_prefix('M')?;
        let generation = model.parse().ok().filter(|g| *g > 0)?;
        let variant = words.next().map(str::to_owned);
        Some(Self {
            generation,
            variant,
        })
    }
}

impl fmt::Display for AppleChip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Apple M{}", self.generation)?;
        if let Some(variant) = &self.variant {
            write!(f, " {variant}")?;
        }
        Ok(())
    }
}

/// macOS product version, e.g. `26.5.2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct MacVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl MacVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parse `sw_vers -productVersion` output; missing minor/patch default to 0.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        let patch = parts.next().map_or(Some(0), |p| p.parse().ok())?;
        Some(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for MacVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// An imported Game Porting Toolkit (D3DMetal) installation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GptkInfo {
    pub dir: PathBuf,
    pub version: Option<String>,
}

impl GptkInfo {
    /// A GPTK import is the extracted `lib/external/` tree plus an optional `VERSION` file.
    pub fn probe(dir: &Path) -> Option<Self> {
        if !dir.join("lib").join("external").is_dir() {
            return None;
        }
        let version = std::fs::read_to_string(dir.join("VERSION"))
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty());
        Some(Self {
            dir: dir.to_owned(),
            version,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SystemInfo {
    /// CPU architecture of this binary (`aarch64`, `x86_64`).
    pub arch: String,
    /// Raw `machdep.cpu.brand_string`.
    pub cpu_brand: Option<String>,
    pub chip: Option<AppleChip>,
    pub macos: Option<MacVersion>,
    pub rosetta_installed: bool,
    /// Path reported by `xcode-select -p`, if the Command Line Tools are installed.
    pub xcode_clt: Option<PathBuf>,
    pub gptk: Option<GptkInfo>,
}

impl SystemInfo {
    /// Probe the running system. Never fails: anything undetectable is `None`/`false`.
    pub fn detect(gptk_dir: &Path) -> Self {
        let cpu_brand = command_output("sysctl", &["-n", "machdep.cpu.brand_string"]);
        Self {
            arch: std::env::consts::ARCH.to_owned(),
            chip: cpu_brand.as_deref().and_then(AppleChip::parse),
            cpu_brand,
            macos: command_output("sw_vers", &["-productVersion"])
                .as_deref()
                .and_then(MacVersion::parse),
            rosetta_installed: rosetta_installed(),
            xcode_clt: command_output("xcode-select", &["-p"]).map(PathBuf::from),
            gptk: GptkInfo::probe(gptk_dir),
        }
    }
}

/// Run a command and return trimmed stdout, or `None` on any failure / empty output.
fn command_output(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apple_chips() {
        let chip = AppleChip::parse("Apple M1 Pro").unwrap();
        assert_eq!(chip.generation, 1);
        assert_eq!(chip.variant.as_deref(), Some("Pro"));
        assert_eq!(chip.to_string(), "Apple M1 Pro");

        let base = AppleChip::parse("Apple M4").unwrap();
        assert_eq!((base.generation, base.variant), (4, None));
    }

    #[test]
    fn rejects_non_apple_chips() {
        assert_eq!(AppleChip::parse("Intel(R) Core(TM) i9-9980HK"), None);
        assert_eq!(AppleChip::parse("Apple"), None);
        assert_eq!(AppleChip::parse("Apple A18"), None);
        assert_eq!(AppleChip::parse("Apple M0"), None);
        assert_eq!(AppleChip::parse(""), None);
    }

    #[test]
    fn parses_macos_versions() {
        assert_eq!(
            MacVersion::parse("26.5.2\n"),
            Some(MacVersion::new(26, 5, 2))
        );
        assert_eq!(MacVersion::parse("15.1"), Some(MacVersion::new(15, 1, 0)));
        assert_eq!(MacVersion::parse("14"), Some(MacVersion::new(14, 0, 0)));
        assert_eq!(MacVersion::parse("abc"), None);
        assert_eq!(MacVersion::parse("15.x"), None);
        assert!(MacVersion::new(15, 0, 0) < MacVersion::new(15, 0, 1));
        assert!(MacVersion::new(14, 9, 9) < MacVersion::new(15, 0, 0));
    }

    #[test]
    fn gptk_probe_requires_lib_external() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(GptkInfo::probe(tmp.path()), None);

        std::fs::create_dir_all(tmp.path().join("lib/external")).unwrap();
        assert_eq!(GptkInfo::probe(tmp.path()).unwrap().version, None);

        std::fs::write(tmp.path().join("VERSION"), "3.0\n").unwrap();
        assert_eq!(
            GptkInfo::probe(tmp.path()).unwrap().version.as_deref(),
            Some("3.0")
        );
    }
}
