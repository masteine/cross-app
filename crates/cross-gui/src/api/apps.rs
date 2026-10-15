//! Program shortcuts in a bottle (`Bottle.apps`) and starting programs and
//! Wine tools in the background with their output in the bottle's `logs/`.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use cross_bottle::launch::{self, LaunchRequest, Tool};
use cross_bottle::Runtime;
use cross_core::config::BottleStore;
use cross_core::model::{App, Bottle, LaunchProfile};
use cross_core::paths::AppDirs;
use cross_steam::STEAM_EXE_WIN;
use serde::Serialize;

use crate::{Error, Result};

/// What can be dropped on the window or picked as a program.
pub const PROGRAM_EXTENSIONS: [&str; 5] = ["exe", "msi", "lnk", "bat", "cmd"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppKind {
    /// A program shortcut: started with `wine <path>`.
    Program,
    /// The Steam client added by `cross steam install`: started with its patches.
    SteamClient,
    /// Per-game Steam settings (`cross steam profile`), not a launchable shortcut.
    SteamProfile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppView {
    pub name: String,
    pub path: String,
    pub kind: AppKind,
    pub steam_appid: Option<u32>,
}

fn kind_of(app: &App) -> AppKind {
    match (app.steam_appid, app.path == Path::new(STEAM_EXE_WIN)) {
        (Some(_), _) => AppKind::SteamProfile,
        (None, true) => AppKind::SteamClient,
        (None, false) => AppKind::Program,
    }
}

pub fn views(bottle: &Bottle) -> Vec<AppView> {
    bottle
        .apps
        .iter()
        .map(|a| AppView {
            name: a.name.clone(),
            path: a.path.display().to_string(),
            kind: kind_of(a),
            steam_appid: a.steam_appid,
        })
        .collect()
}

pub fn is_program(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| PROGRAM_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// `C:\...` for a file inside the bottle's `drive_c`, so the shortcut survives
/// `cross bottle clone`/`rename`; `None` for files elsewhere on the Mac.
fn windows_path_in_prefix(prefix: &Path, file: &Path) -> Option<String> {
    let drive_c = prefix.join("drive_c").canonicalize().ok()?;
    let rel = file.strip_prefix(&drive_c).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(format!(r"C:\{}", parts.join(r"\")))
}

/// The host file behind a `C:\...` path of this bottle, if it exists.
fn host_path_in_prefix(prefix: &Path, windows: &str) -> Option<PathBuf> {
    let rel = windows
        .strip_prefix("C:\\")
        .or_else(|| windows.strip_prefix("c:\\"))?;
    let mut path = prefix.join("drive_c");
    for part in rel.split('\\').filter(|p| !p.is_empty()) {
        if part == ".." {
            return None;
        }
        path.push(part);
    }
    path.is_file().then_some(path)
}

/// Add a shortcut for a program on this Mac (drag-and-drop or "Add program").
pub fn add(dirs: &AppDirs, bottle_id: &str, file: &Path, name: Option<&str>) -> Result<AppView> {
    let store = BottleStore::new(dirs);
    let mut bottle = store.load(bottle_id)?;
    if !file.is_file() {
        return Err(Error::InvalidInput(format!(
            "{} is not a file",
            file.display()
        )));
    }
    if !is_program(file) {
        return Err(Error::InvalidInput(format!(
            "{} is not a Windows program (expected {})",
            file.display(),
            PROGRAM_EXTENSIONS.join(", ")
        )));
    }
    let file = file.canonicalize().map_err(|e| Error::io(file, e))?;
    let stored = windows_path_in_prefix(&dirs.bottle_prefix(bottle_id), &file)
        .unwrap_or_else(|| file.display().to_string());
    if bottle
        .apps
        .iter()
        .any(|a| a.path.as_os_str() == stored.as_str())
    {
        return Err(Error::InvalidInput(format!(
            "{stored} is already in bottle {bottle_id}"
        )));
    }
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
        .or_else(|| file.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| stored.clone());
    let app = App {
        name,
        path: stored.into(),
        steam_appid: None,
        profile: LaunchProfile::default(),
    };
    tracing::info!(bottle = bottle_id, app = %app.name, path = %app.path.display(), "shortcut added");
    let view = AppView {
        name: app.name.clone(),
        path: app.path.display().to_string(),
        kind: kind_of(&app),
        steam_appid: None,
    };
    bottle.apps.push(app);
    store.save(&bottle)?;
    Ok(view)
}

/// Remove a program shortcut (Steam entries are managed by `cross steam`).
pub fn remove(dirs: &AppDirs, bottle_id: &str, path: &str) -> Result<()> {
    let store = BottleStore::new(dirs);
    let mut bottle = store.load(bottle_id)?;
    let before = bottle.apps.len();
    bottle
        .apps
        .retain(|a| !(kind_of(a) == AppKind::Program && a.path.as_os_str() == path));
    if bottle.apps.len() == before {
        return Err(Error::InvalidInput(format!(
            "no program shortcut {path} in bottle {bottle_id}"
        )));
    }
    store.save(&bottle)?;
    Ok(())
}

/// A program started in the background.
#[derive(Debug)]
pub struct Spawned {
    pub child: Child,
    pub log: PathBuf,
    pub label: String,
}

/// Log file name for a program: `My Game.exe` -> `my_game.log`.
pub fn log_name(label: &str) -> String {
    let stem = Path::new(label.rsplit(['\\', '/']).next().unwrap_or(label))
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let slug: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if slug.is_empty() {
        "program.log".to_owned()
    } else {
        format!("{slug}.log")
    }
}

fn open_log(dirs: &AppDirs, bottle_id: &str, label: &str) -> Result<(PathBuf, File)> {
    let logs = dirs.bottle_logs_dir(bottle_id);
    fs::create_dir_all(&logs).map_err(|e| Error::io(&logs, e))?;
    let path = logs.join(log_name(label));
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| Error::io(&path, e))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    writeln!(
        file,
        "=== cross-app: starting {label} (unix time {now}) ==="
    )
    .map_err(|e| Error::io(&path, e))?;
    Ok((path, file))
}

fn spawn_command(
    mut cmd: std::process::Command,
    log: PathBuf,
    file: File,
    label: String,
) -> Result<Spawned> {
    let err = file.try_clone().map_err(|e| Error::io(&log, e))?;
    cmd.stdin(Stdio::null()).stdout(file).stderr(err);
    let child = cmd.spawn().map_err(|e| {
        Error::from(cross_bottle::Error::Wine {
            what: "launch".into(),
            message: format!("could not start {label}: {e}"),
        })
    })?;
    Ok(Spawned { child, log, label })
}

/// Start a program of the bottle (a shortcut's path, or any path or Windows
/// name) without waiting for it. A shortcut's launch profile is applied.
pub fn spawn_program(dirs: &AppDirs, bottle_id: &str, target: &str) -> Result<Spawned> {
    let (bottle, runtime) = Runtime::open(dirs, bottle_id)?;
    if target == STEAM_EXE_WIN {
        return Err(Error::InvalidInput(
            "start Steam with its own button so its patches are applied".into(),
        ));
    }
    let profile = bottle
        .apps
        .iter()
        .find(|a| a.steam_appid.is_none() && a.path.as_os_str() == target)
        .map(|a| a.profile.clone())
        .unwrap_or_default();
    // A `C:\` path inside this bottle runs from its own folder, like a host path.
    let resolved = host_path_in_prefix(runtime.prefix(), target)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| target.to_owned());
    let request = LaunchRequest {
        target: resolved,
        profile,
        ..Default::default()
    };
    let cmd = launch::build_command(&runtime, &request)?;
    let (log, file) = open_log(dirs, bottle_id, target)?;
    tracing::info!(bottle = bottle_id, program = target, log = %log.display(), "starting program from the GUI");
    spawn_command(cmd, log, file, target.to_owned())
}

/// Start `winecfg`, `regedit`, `cmd` or `taskmgr` without waiting.
pub fn spawn_tool(dirs: &AppDirs, bottle_id: &str, tool: Tool) -> Result<Spawned> {
    let (_, runtime) = Runtime::open(dirs, bottle_id)?;
    let cmd = launch::tool_command(&runtime, tool);
    let (log, file) = open_log(dirs, bottle_id, tool.program())?;
    tracing::info!(
        bottle = bottle_id,
        tool = tool.program(),
        "starting Wine tool from the GUI"
    );
    spawn_command(cmd, log, file, tool.program().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::test_support::fake_bottle;

    #[test]
    fn dropped_programs_become_shortcuts() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        fake_bottle(&dirs, "demo", "wine-a");
        let exe = tmp.path().join("Setup Tool.EXE");
        fs::write(&exe, "MZ").unwrap();

        let view = add(&dirs, "demo", &exe, None).unwrap();
        assert_eq!(view.name, "Setup Tool");
        assert_eq!(view.kind, AppKind::Program);
        assert_eq!(view.path, exe.canonicalize().unwrap().display().to_string());

        let err = add(&dirs, "demo", &exe, Some("Again")).unwrap_err();
        assert!(err.to_string().contains("already in bottle"), "{err}");

        let bottle = BottleStore::new(&dirs).load("demo").unwrap();
        assert_eq!(bottle.apps.len(), 1);
        assert_eq!(views(&bottle), std::slice::from_ref(&view));

        remove(&dirs, "demo", &view.path).unwrap();
        assert!(BottleStore::new(&dirs)
            .load("demo")
            .unwrap()
            .apps
            .is_empty());
        assert!(remove(&dirs, "demo", &view.path).is_err());
    }

    #[test]
    fn programs_inside_drive_c_are_stored_as_windows_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        fake_bottle(&dirs, "demo", "wine-a");
        let dir = dirs.bottle_prefix("demo").join("drive_c/Program Files/App");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("app.exe"), "MZ").unwrap();

        let view = add(&dirs, "demo", &dir.join("app.exe"), Some("  My App ")).unwrap();
        assert_eq!(view.path, r"C:\Program Files\App\app.exe");
        assert_eq!(view.name, "My App");
        assert_eq!(
            host_path_in_prefix(&dirs.bottle_prefix("demo"), &view.path),
            Some(dir.join("app.exe"))
        );
        assert_eq!(
            host_path_in_prefix(&dirs.bottle_prefix("demo"), r"C:\..\..\etc\passwd"),
            None
        );
    }

    #[test]
    fn only_windows_programs_are_accepted() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        fake_bottle(&dirs, "demo", "wine-a");
        let doc = tmp.path().join("notes.txt");
        fs::write(&doc, "").unwrap();
        assert!(add(&dirs, "demo", &doc, None).is_err());
        assert!(add(&dirs, "demo", &tmp.path().join("gone.exe"), None).is_err());
        assert!(add(&dirs, "nope", &doc, None).is_err());
        for ok in ["a.exe", "b.MSI", "c.lnk", "d.bat", "e.cmd"] {
            assert!(is_program(Path::new(ok)), "{ok}");
        }
        assert!(!is_program(Path::new("x.dmg")));
    }

    #[test]
    fn steam_entries_are_not_program_shortcuts() {
        let mut bottle = Bottle::new("s", "S").unwrap();
        bottle.apps.push(App {
            name: "Steam".into(),
            path: STEAM_EXE_WIN.into(),
            steam_appid: None,
            profile: LaunchProfile::default(),
        });
        bottle.apps.push(App {
            name: "Game".into(),
            path: STEAM_EXE_WIN.into(),
            steam_appid: Some(220),
            profile: LaunchProfile::default(),
        });
        let kinds: Vec<_> = views(&bottle).into_iter().map(|v| v.kind).collect();
        assert_eq!(kinds, [AppKind::SteamClient, AppKind::SteamProfile]);
    }

    #[test]
    fn log_names_are_safe_file_names() {
        assert_eq!(log_name("My Game.exe"), "my_game.log");
        assert_eq!(log_name(r"C:\Program Files\App\Setup-1.EXE"), "setup-1.log");
        assert_eq!(log_name("/Users/me/x/../run.bat"), "run.log");
        assert_eq!(log_name("winecfg"), "winecfg.log");
        assert_eq!(log_name(""), "program.log");
    }

    #[test]
    fn spawned_programs_log_into_the_bottle() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        fake_bottle(&dirs, "demo", "wine-a");
        let exe = tmp.path().join("game.exe");
        fs::write(&exe, "MZ").unwrap();
        let view = add(&dirs, "demo", &exe, None).unwrap();

        let mut spawned = spawn_program(&dirs, "demo", &view.path).unwrap();
        assert!(spawned.child.wait().unwrap().success());
        assert_eq!(spawned.log, dirs.bottle_logs_dir("demo").join("game.log"));
        let log = fs::read_to_string(&spawned.log).unwrap();
        assert!(log.contains("=== cross-app: starting"), "{log}");
        assert!(log.contains(&format!("ARG:{}", view.path)), "{log}");
        assert!(
            log.contains(&format!(
                "CWD:{}",
                exe.canonicalize().unwrap().parent().unwrap().display()
            )),
            "{log}"
        );

        let mut tool = spawn_tool(&dirs, "demo", Tool::Regedit).unwrap();
        assert!(tool.child.wait().unwrap().success());
        let log = fs::read_to_string(&tool.log).unwrap();
        assert!(log.contains("ARG:regedit"), "{log}");

        let err = spawn_program(&dirs, "demo", STEAM_EXE_WIN).unwrap_err();
        assert!(err.to_string().contains("Steam"), "{err}");
    }
}
