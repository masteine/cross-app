//! The games Steam has installed in a bottle.
//!
//! Steam lists its library folders in `steamapps/libraryfolders.vdf` (Windows
//! paths) and describes each installed app in `<library>/steamapps/appmanifest_<appid>.acf`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::vdf::{self, Vdf};
use crate::{Error, Result, STEAM_DIR_IN_PREFIX};

/// Steamworks Common Redistributables: installed by Steam, not a game.
const TOOL_APPIDS: [u32; 1] = [228_980];

// `StateFlags` bits (Steam's EAppState).
const STATE_UPDATE_REQUIRED: u32 = 2;
const STATE_FULLY_INSTALLED: u32 = 4;
const STATE_UPDATE_RUNNING: u32 = 256;
const STATE_UPDATE_PAUSED: u32 = 512;
const STATE_UPDATE_STARTED: u32 = 1024;
const STATE_DOWNLOADING: u32 = 1_048_576;
const STATE_ANY_UPDATE: u32 =
    STATE_UPDATE_RUNNING | STATE_UPDATE_PAUSED | STATE_UPDATE_STARTED | STATE_DOWNLOADING;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GameState {
    Installed,
    UpdateRequired,
    Updating,
    /// Anything else (partially installed, validating, ...); see `state_flags`.
    Other,
}

impl GameState {
    pub fn from_flags(flags: u32) -> Self {
        if flags & STATE_ANY_UPDATE != 0 {
            GameState::Updating
        } else if flags & STATE_UPDATE_REQUIRED != 0 {
            GameState::UpdateRequired
        } else if flags & STATE_FULLY_INSTALLED != 0 {
            GameState::Installed
        } else {
            GameState::Other
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Game {
    pub appid: u32,
    pub name: String,
    /// Where the game's files are, on this Mac.
    pub install_dir: PathBuf,
    pub size_on_disk: u64,
    pub state: GameState,
    pub state_flags: u32,
    /// Steam redistributables and similar, not a playable game.
    pub is_tool: bool,
}

/// The Steam folder inside a prefix.
pub fn steam_dir(prefix: &Path) -> PathBuf {
    prefix.join(STEAM_DIR_IN_PREFIX)
}

/// Map a Windows path (`D:\Games\Lib`) to the host through the prefix's
/// `dosdevices/<drive>:` links, which is how Wine itself resolves drives.
pub fn windows_to_host(prefix: &Path, windows_path: &str) -> Option<PathBuf> {
    let bytes = windows_path.as_bytes();
    if bytes.len() < 2 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' {
        return None;
    }
    let drive = format!("{}:", (bytes[0] as char).to_ascii_lowercase());
    let mut path = prefix.join("dosdevices").join(drive);
    for part in windows_path[2..]
        .split(['\\', '/'])
        .filter(|p| !p.is_empty())
    {
        if part == ".." {
            return None;
        }
        path.push(part);
    }
    Some(path)
}

/// Library folders on the host, the main Steam folder first. Folders listed in
/// `libraryfolders.vdf` that do not map or do not exist are skipped.
pub fn library_folders(prefix: &Path) -> Result<Vec<PathBuf>> {
    let main = steam_dir(prefix);
    let mut folders = vec![main.clone()];
    let file = main.join("steamapps/libraryfolders.vdf");
    let doc = match read_vdf(&file) {
        Ok(doc) => doc,
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(folders)
        }
        Err(e) => return Err(e),
    };
    let root = doc
        .get("libraryfolders")
        .or_else(|| doc.get("LibraryFolders"))
        .cloned()
        .unwrap_or(Vdf::Map(vec![]));

    for (key, value) in root.entries() {
        if key.parse::<u32>().is_err() {
            continue; // e.g. "TimeNextStatsReport" in the old format
        }
        // New format: "0" { "path" "..." }; old format: "1" "D:\\SteamLibrary".
        let windows = match value {
            Vdf::Map(_) => value.get_str("path"),
            Vdf::Str(path) => Some(path.as_str()),
        };
        let Some(host) = windows.and_then(|w| windows_to_host(prefix, w)) else {
            continue;
        };
        if !host.is_dir() || folders.iter().any(|f| same_dir(f, &host)) {
            continue;
        }
        // Show the real location (e.g. /Volumes/Games/SteamLibrary), not the dosdevices link.
        folders.push(host.canonicalize().unwrap_or(host));
    }
    Ok(folders)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Every app with a manifest in any library folder, sorted by name.
/// Manifests that cannot be read are skipped with a warning.
pub fn games(prefix: &Path) -> Result<Vec<Game>> {
    let mut games = Vec::new();
    for folder in library_folders(prefix)? {
        let steamapps = folder.join("steamapps");
        let Ok(entries) = fs::read_dir(&steamapps) else {
            continue;
        };
        let mut manifests: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("appmanifest_") && n.ends_with(".acf"))
            })
            .collect();
        manifests.sort();
        for manifest in manifests {
            match read_manifest(&manifest, &steamapps) {
                Ok(game) if !games.iter().any(|g: &Game| g.appid == game.appid) => games.push(game),
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(path = %manifest.display(), error = %e, "skipping app manifest")
                }
            }
        }
    }
    games.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.appid.cmp(&b.appid))
    });
    Ok(games)
}

fn read_manifest(path: &Path, steamapps: &Path) -> Result<Game> {
    let doc = read_vdf(path)?;
    let invalid = |message: &str| Error::Vdf {
        path: path.to_owned(),
        line: 0,
        message: message.to_owned(),
    };
    let app = doc
        .get("AppState")
        .ok_or_else(|| invalid("no AppState section"))?;
    let appid: u32 = app
        .get_str("appid")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| invalid("missing or invalid appid"))?;
    let name = app
        .get_str("name")
        .filter(|n| !n.is_empty())
        .map_or_else(|| format!("App {appid}"), str::to_owned);
    let installdir = app.get_str("installdir").unwrap_or_default();
    if installdir.contains(['/', '\\']) || installdir == ".." {
        return Err(invalid("installdir must be a folder name"));
    }
    let flags: u32 = app
        .get_str("StateFlags")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    Ok(Game {
        appid,
        name,
        install_dir: steamapps.join("common").join(installdir),
        size_on_disk: app
            .get_str("SizeOnDisk")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        state: GameState::from_flags(flags),
        state_flags: flags,
        is_tool: TOOL_APPIDS.contains(&appid),
    })
}

fn read_vdf(path: &Path) -> Result<Vdf> {
    let text = fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
    vdf::parse(&text).map_err(|e| Error::Vdf {
        path: path.to_owned(),
        line: e.line,
        message: e.message,
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    fn manifest(appid: u32, name: &str, installdir: &str, flags: u32, size: u64) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{appid}\"\n\t\"name\"\t\t\"{name}\"\n\t\"StateFlags\"\t\t\"{flags}\"\n\
             \t\"installdir\"\t\t\"{installdir}\"\n\t\"SizeOnDisk\"\t\t\"{size}\"\n}}\n"
        )
    }

    /// A prefix with Steam on C: and a second library on D: (a separate host folder).
    fn prefix_with_two_libraries() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("prefix");
        let d_drive = tmp.path().join("external-disk");
        fs::create_dir_all(prefix.join("dosdevices")).unwrap();
        fs::create_dir_all(prefix.join("drive_c")).unwrap();
        fs::create_dir_all(d_drive.join("SteamLibrary/steamapps")).unwrap();
        symlink("../drive_c", prefix.join("dosdevices/c:")).unwrap();
        symlink(&d_drive, prefix.join("dosdevices/d:")).unwrap();

        let steamapps = steam_dir(&prefix).join("steamapps");
        fs::create_dir_all(&steamapps).unwrap();
        fs::write(
            steamapps.join("libraryfolders.vdf"),
            r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps" { "220" "6599921433" }
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps" { "1091500" "1" }
	}
	"2"
	{
		"path"		"Q:\\Gone"
	}
}
"#,
        )
        .unwrap();
        (tmp, prefix, d_drive)
    }

    #[test]
    fn maps_windows_paths_through_dosdevices() {
        let prefix = Path::new("/p");
        assert_eq!(
            windows_to_host(prefix, r"C:\Program Files (x86)\Steam").unwrap(),
            Path::new("/p/dosdevices/c:/Program Files (x86)/Steam")
        );
        assert_eq!(
            windows_to_host(prefix, "d:/Games//Lib\\").unwrap(),
            Path::new("/p/dosdevices/d:/Games/Lib")
        );
        assert_eq!(windows_to_host(prefix, r"C:\a\..\..\etc"), None);
        assert_eq!(windows_to_host(prefix, r"\\server\share"), None);
        assert_eq!(windows_to_host(prefix, "relative"), None);
    }

    #[test]
    fn lists_games_across_libraries() {
        let (_tmp, prefix, d_drive) = prefix_with_two_libraries();
        let c_apps = steam_dir(&prefix).join("steamapps");
        fs::write(
            c_apps.join("appmanifest_220.acf"),
            manifest(220, "Half-Life 2", "Half-Life 2", 4, 6_599_921_433),
        )
        .unwrap();
        fs::write(
            c_apps.join("appmanifest_228980.acf"),
            manifest(
                228_980,
                "Steamworks Common Redistributables",
                "Steamworks Shared",
                4,
                1,
            ),
        )
        .unwrap();
        fs::write(c_apps.join("appmanifest_bad.acf"), "not vdf {").unwrap();
        fs::write(c_apps.join("libraryfolders.bak"), "").unwrap();
        let d_apps = d_drive.join("SteamLibrary/steamapps");
        fs::write(
            d_apps.join("appmanifest_1091500.acf"),
            manifest(1_091_500, "Cyberpunk 2077", "Cyberpunk 2077", 6, 70),
        )
        .unwrap();

        let folders = library_folders(&prefix).unwrap();
        assert_eq!(folders.len(), 2, "{folders:?}");

        let games = games(&prefix).unwrap();
        let summary: Vec<_> = games
            .iter()
            .map(|g| (g.appid, g.name.as_str(), g.state, g.is_tool))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    1_091_500,
                    "Cyberpunk 2077",
                    GameState::UpdateRequired,
                    false
                ),
                (220, "Half-Life 2", GameState::Installed, false),
                (
                    228_980,
                    "Steamworks Common Redistributables",
                    GameState::Installed,
                    true
                ),
            ]
        );
        assert_eq!(games[1].size_on_disk, 6_599_921_433);
        assert_eq!(games[1].install_dir, c_apps.join("common/Half-Life 2"));
        assert_eq!(
            games[0].install_dir,
            d_apps.canonicalize().unwrap().join("common/Cyberpunk 2077")
        );
    }

    #[test]
    fn old_library_format_and_missing_files() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("prefix");
        // No Steam at all: just the main folder, no games.
        assert_eq!(library_folders(&prefix).unwrap(), vec![steam_dir(&prefix)]);
        assert!(games(&prefix).unwrap().is_empty());

        let other = tmp.path().join("lib2");
        fs::create_dir_all(other.join("steamapps")).unwrap();
        fs::create_dir_all(prefix.join("dosdevices")).unwrap();
        symlink(&other, prefix.join("dosdevices/e:")).unwrap();
        fs::create_dir_all(steam_dir(&prefix).join("steamapps")).unwrap();
        fs::write(
            steam_dir(&prefix).join("steamapps/libraryfolders.vdf"),
            "\"LibraryFolders\"\n{\n\t\"TimeNextStatsReport\"\t\"1\"\n\t\"1\"\t\"E:\\\\\"\n}\n",
        )
        .unwrap();
        assert_eq!(library_folders(&prefix).unwrap().len(), 2);
    }

    #[test]
    fn hostile_installdir_is_rejected() {
        let (_tmp, prefix, _) = prefix_with_two_libraries();
        let apps = steam_dir(&prefix).join("steamapps");
        fs::write(
            apps.join("appmanifest_1.acf"),
            manifest(1, "Evil", "../../..", 4, 0),
        )
        .unwrap();
        assert!(games(&prefix).unwrap().is_empty());
    }

    #[test]
    fn state_flags() {
        assert_eq!(GameState::from_flags(4), GameState::Installed);
        assert_eq!(GameState::from_flags(6), GameState::UpdateRequired);
        assert_eq!(GameState::from_flags(4 | 1024), GameState::Updating);
        assert_eq!(GameState::from_flags(1_048_576 | 2), GameState::Updating);
        assert_eq!(GameState::from_flags(0), GameState::Other);
    }
}
