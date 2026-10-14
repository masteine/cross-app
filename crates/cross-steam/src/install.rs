//! `cross steam install`: put the Windows Steam client into a bottle.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cross_bottle::exec::run_captured;
use cross_bottle::processes::StopMode;
use cross_bottle::Runtime;
use cross_core::config::BottleStore;
use cross_core::model::{App, SteamRecord};
use cross_core::paths::AppDirs;
use cross_engine::catalog::Archive;
use cross_engine::download::fetch_verified;
use cross_engine::Progress;

use crate::library::steam_dir;
use crate::patches::SteamPatches;
use crate::{Error, Result, STEAM_EXE_WIN};

const SETUP_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(60);
const SETUP_FILE: &str = "SteamSetup.exe";
const STEAM_APP_NAME: &str = "Steam";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallReport {
    pub installer_sha256: String,
    pub patches_revision: u32,
}

/// Whether `steam.exe` is present in the bottle.
pub fn is_installed(prefix: &Path) -> bool {
    let dir = steam_dir(prefix);
    dir.join("steam.exe").is_file() || dir.join("Steam.exe").is_file()
}

/// Download the pinned `SteamSetup.exe` (cached, SHA-256 verified), install it
/// silently, apply the registry patches and record Steam in `bottle.toml`.
pub fn install(
    dirs: &AppDirs,
    bottle_id: &str,
    force: bool,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<InstallReport> {
    let (_, runtime) = Runtime::open(dirs, bottle_id)?;
    if is_installed(runtime.prefix()) && !force {
        return Err(Error::AlreadyInstalled(bottle_id.to_owned()));
    }
    let (patches, _) = SteamPatches::load(dirs)?;

    let archive = Archive {
        url: patches.installer.url.clone(),
        sha256: patches.installer.sha256.clone(),
        size: patches.installer.size,
    };
    let cached = fetch_verified(&archive, SETUP_FILE, &dirs.downloads_dir(), progress)?;
    let work = tempfile::tempdir().map_err(|e| Error::io(std::env::temp_dir(), e))?;
    let setup = work.path().join(SETUP_FILE);
    fs::copy(&cached, &setup).map_err(|e| Error::io(&setup, e))?;

    tracing::info!(bottle = bottle_id, sha256 = %archive.sha256, "running SteamSetup.exe");
    let mut cmd = runtime.wine_command();
    cmd.arg(&setup)
        .args(&patches.installer.args)
        .current_dir(work.path());
    let out = run_captured(cmd, "SteamSetup.exe", SETUP_TIMEOUT)?;
    if !out.status.success() {
        return Err(Error::Setup(format!(
            "SteamSetup.exe failed: {}",
            out.message()
        )));
    }
    settle(&runtime)?;
    if !is_installed(runtime.prefix()) {
        return Err(Error::Setup(
            "SteamSetup.exe finished but steam.exe is not in Program Files (x86)\\Steam".into(),
        ));
    }

    patches.apply_registry(&runtime)?;
    let mut wait = runtime.wineserver_command();
    wait.arg("-w");
    run_captured(wait, "wineserver -w", SETTLE_TIMEOUT)?;

    let store = BottleStore::new(dirs);
    let mut bottle = store.load(bottle_id)?;
    bottle.steam = Some(SteamRecord {
        patches_revision: patches.revision,
        installer_sha256: archive.sha256.clone(),
        installed_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    });
    if !bottle
        .apps
        .iter()
        .any(|a| a.steam_appid.is_none() && a.name == STEAM_APP_NAME)
    {
        bottle.apps.push(App {
            name: STEAM_APP_NAME.to_owned(),
            path: STEAM_EXE_WIN.into(),
            steam_appid: None,
            profile: Default::default(),
        });
    }
    store.save(&bottle)?;

    Ok(InstallReport {
        installer_sha256: archive.sha256,
        patches_revision: patches.revision,
    })
}

/// After setup, wait for Wine to go idle. The installer may start Steam itself
/// (its first-run update would keep the bottle busy for minutes), so a Steam we
/// did not ask for is stopped; it resumes its update on the next start.
fn settle(runtime: &Runtime) -> Result<()> {
    let running = runtime.processes()?;
    if running
        .iter()
        .any(|p| p.name.eq_ignore_ascii_case("steam.exe"))
    {
        tracing::info!("SteamSetup started Steam; stopping it until the user starts it");
        runtime.stop(StopMode::Force)?;
        return Ok(());
    }
    let mut wait = runtime.wineserver_command();
    wait.arg("-w");
    match run_captured(wait, "wineserver -w", SETTLE_TIMEOUT) {
        Ok(_) => Ok(()),
        Err(cross_bottle::Error::Timeout { .. }) => {
            runtime.stop(StopMode::Force)?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use cross_core::model::{Bottle, GraphicsBackend, Translation};
    use cross_engine::manager::{InstalledEngine, ENGINE_FILE};
    use sha2::{Digest, Sha256};

    use super::*;

    /// `wine <x>/SteamSetup.exe /S` creates steam.exe unless `setup-mode` says otherwise.
    const FAKE_WINE: &str = r#"#!/bin/sh
here="$(dirname "$0")"
echo "ARGS:$*" >> "$here/calls.log"
case "$1" in
  reg) [ "$2" = "add" ] && echo "$3|$5|$7|$9" >> "$here/reg.db" ;;
  *SteamSetup.exe)
    mode="$(cat "$here/setup-mode" 2>/dev/null)"
    [ "$mode" = "fail" ] && { echo "setup crashed" >&2; exit 2; }
    [ "$mode" = "nothing" ] && exit 0
    mkdir -p "$WINEPREFIX/drive_c/Program Files (x86)/Steam"
    echo exe > "$WINEPREFIX/drive_c/Program Files (x86)/Steam/steam.exe" ;;
esac
exit 0
"#;

    struct Fixture {
        _tmp: tempfile::TempDir,
        dirs: AppDirs,
        bin: std::path::PathBuf,
        sha: String,
    }

    fn script(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        let engine_dir = dirs.engine_dir("eng");
        let bin = engine_dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        script(&bin.join("wine"), FAKE_WINE);
        script(&bin.join("wineserver"), "#!/bin/sh\nexit 0\n");
        let engine = InstalledEngine {
            id: "eng".into(),
            name: "Eng".into(),
            wine_version: "1".into(),
            translation: Translation::Rosetta,
            wine_bin: "bin/wine".into(),
            backends: vec![GraphicsBackend::D3dMetal],
            components: vec![],
            source_sha256: "0".repeat(64),
            installed_unix: 0,
        };
        fs::write(
            engine_dir.join(ENGINE_FILE),
            toml::to_string(&engine).unwrap(),
        )
        .unwrap();
        let mut bottle = Bottle::new("games", "Games").unwrap();
        bottle.engine = Some("eng".into());
        BottleStore::new(&dirs).save(&bottle).unwrap();
        fs::create_dir_all(dirs.bottle_prefix("games")).unwrap();

        // A local patch file pinning our fake installer, which is pre-seeded in the cache.
        let content = b"fake SteamSetup";
        let sha = format!("{:x}", Sha256::digest(content));
        let patches = include_str!("../steam-patches.toml")
            .replace(
                "7d3654531c32d941b8cae81c4137fc542172bfa9635f169cb392f245a0a12bcb",
                &sha,
            )
            .replace("size = 2380800", "size = 15")
            .replace("revision = 1", "revision = 5");
        fs::write(dirs.root().join("steam-patches.toml"), patches).unwrap();
        fs::create_dir_all(dirs.downloads_dir()).unwrap();
        fs::write(
            dirs.downloads_dir().join(format!("{sha}-SteamSetup.exe")),
            content,
        )
        .unwrap();

        Fixture {
            _tmp: tmp,
            dirs,
            bin,
            sha,
        }
    }

    #[test]
    fn installs_applies_patches_and_records_steam() {
        let fx = fixture();
        let report = install(&fx.dirs, "games", false, &mut |_| {}).unwrap();
        assert_eq!(
            report,
            InstallReport {
                installer_sha256: fx.sha.clone(),
                patches_revision: 5
            }
        );
        assert!(is_installed(&fx.dirs.bottle_prefix("games")));

        let calls = fs::read_to_string(fx.bin.join("calls.log")).unwrap();
        assert!(calls.contains("SteamSetup.exe /S"), "{calls}");
        let db = fs::read_to_string(fx.bin.join("reg.db")).unwrap();
        assert!(
            db.contains(r"HKCU\Software\Wine\DllOverrides|gameoverlayrenderer|REG_SZ|"),
            "{db}"
        );
        assert!(
            db.contains(r"HKCU\Software\Wine\WineDbg|ShowCrashDialog|REG_DWORD|0"),
            "{db}"
        );

        let bottle = BottleStore::new(&fx.dirs).load("games").unwrap();
        let record = bottle.steam.unwrap();
        assert_eq!(
            (record.patches_revision, record.installer_sha256.as_str()),
            (5, fx.sha.as_str())
        );
        assert_eq!(bottle.apps.iter().filter(|a| a.name == "Steam").count(), 1);

        // Again: refused, unless forced (and the Steam app is not duplicated).
        assert!(matches!(
            install(&fx.dirs, "games", false, &mut |_| {}).unwrap_err(),
            Error::AlreadyInstalled(_)
        ));
        install(&fx.dirs, "games", true, &mut |_| {}).unwrap();
        let bottle = BottleStore::new(&fx.dirs).load("games").unwrap();
        assert_eq!(bottle.apps.iter().filter(|a| a.name == "Steam").count(), 1);
    }

    #[test]
    fn setup_failures_are_reported_and_nothing_is_recorded() {
        let fx = fixture();
        fs::write(fx.bin.join("setup-mode"), "fail").unwrap();
        let err = install(&fx.dirs, "games", false, &mut |_| {})
            .unwrap_err()
            .to_string();
        assert!(err.contains("setup crashed"), "{err}");

        fs::write(fx.bin.join("setup-mode"), "nothing").unwrap();
        let err = install(&fx.dirs, "games", false, &mut |_| {})
            .unwrap_err()
            .to_string();
        assert!(err.contains("steam.exe is not in"), "{err}");
        assert!(BottleStore::new(&fx.dirs)
            .load("games")
            .unwrap()
            .steam
            .is_none());
    }

    #[test]
    fn a_changed_installer_fails_verification_before_running() {
        let fx = fixture();
        // The cached file no longer matches the pinned hash and the URL is unreachable.
        let cached = fx
            .dirs
            .downloads_dir()
            .join(format!("{}-SteamSetup.exe", fx.sha));
        fs::write(&cached, b"tampered").unwrap();
        assert!(install(&fx.dirs, "games", false, &mut |_| {}).is_err());
        let calls = fs::read_to_string(fx.bin.join("calls.log")).unwrap_or_default();
        assert!(
            !calls.contains("SteamSetup.exe"),
            "setup must not run: {calls}"
        );
    }
}
