//! System status for the first-run wizard and the header: `cross doctor` plus
//! what is already set up.

use cross_core::config::{BottleStore, Config};
use cross_core::doctor::{self, Report};
use cross_core::paths::AppDirs;
use cross_core::system::SystemInfo;
use cross_engine::manager::EngineManager;
use cross_engine::rosetta;
use serde::Serialize;

use crate::Result;

#[derive(Debug, Clone, Serialize)]
pub struct SystemStatus {
    pub system: SystemInfo,
    pub report: Report,
    /// The command the Rosetta step runs, shown before asking for confirmation.
    pub rosetta_command: String,
    pub engines_installed: Vec<String>,
    pub bottles: usize,
    pub default_engine: Option<String>,
    /// No bottle yet: the page opens the first-run wizard.
    pub needs_setup: bool,
    pub data_dir: String,
}

pub fn status(dirs: &AppDirs) -> Result<SystemStatus> {
    let config = Config::load(dirs)?;
    let system = SystemInfo::detect(&config.gptk_dir(dirs));
    evaluate(dirs, &config, system)
}

/// Everything but the probing, so tests can feed a synthetic [`SystemInfo`].
fn evaluate(dirs: &AppDirs, config: &Config, system: SystemInfo) -> Result<SystemStatus> {
    let report = doctor::evaluate(&system);
    let engines_installed = EngineManager::new(dirs)
        .list()?
        .into_iter()
        .map(|e| e.id)
        .collect();
    let bottles = BottleStore::new(dirs).list()?.len();
    Ok(SystemStatus {
        system,
        report,
        rosetta_command: rosetta::command_line(),
        engines_installed,
        bottles,
        default_engine: config.default_engine.clone(),
        needs_setup: bottles == 0,
        data_dir: dirs.root().display().to_string(),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RosettaOutcome {
    AlreadyInstalled,
    Installed,
}

/// Run `softwareupdate --install-rosetta`. The page asks for confirmation first.
pub fn install_rosetta() -> Result<RosettaOutcome> {
    tracing::info!("installing Rosetta 2 from the GUI");
    Ok(match rosetta::install()? {
        rosetta::Outcome::AlreadyInstalled => RosettaOutcome::AlreadyInstalled,
        rosetta::Outcome::Installed => RosettaOutcome::Installed,
    })
}

#[cfg(test)]
mod tests {
    use cross_core::model::Bottle;

    use super::*;

    fn synthetic(rosetta: bool) -> SystemInfo {
        SystemInfo {
            arch: "aarch64".into(),
            cpu_brand: Some("Apple M2".into()),
            chip: cross_core::system::AppleChip::parse("Apple M2"),
            macos: cross_core::system::MacVersion::parse("26.0"),
            rosetta_installed: rosetta,
            xcode_clt: None,
            gptk: None,
        }
    }

    #[test]
    fn empty_data_dir_needs_setup() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let status = evaluate(&dirs, &Config::default(), synthetic(false)).unwrap();
        assert!(status.needs_setup);
        assert!(status.engines_installed.is_empty());
        let rosetta = status
            .report
            .checks
            .iter()
            .find(|c| c.name == "Rosetta 2")
            .unwrap();
        assert_eq!(rosetta.status, doctor::Status::Warn);
        assert_eq!(
            status.rosetta_command,
            "softwareupdate --install-rosetta --agree-to-license"
        );
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["system"]["rosetta_installed"], false);
        assert_eq!(json["report"]["checks"][0]["status"], "ok");
    }

    #[test]
    fn a_bottle_ends_the_wizard() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        BottleStore::new(&dirs)
            .save(&Bottle::new("demo", "Demo").unwrap())
            .unwrap();
        let config = Config {
            default_engine: Some("wine-x".into()),
            ..Config::default()
        };
        let status = evaluate(&dirs, &config, synthetic(true)).unwrap();
        assert!(!status.needs_setup);
        assert_eq!(status.bottles, 1);
        assert_eq!(status.default_engine.as_deref(), Some("wine-x"));
    }
}
