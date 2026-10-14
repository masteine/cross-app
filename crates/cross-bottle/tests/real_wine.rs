//! Integration tests against a real Wine engine.
//!
//! They are `#[ignore]`d because they need an installed engine and take a minute.
//! Install one with `cross engine install wine-devel-11.18` and run:
//!
//! ```text
//! CROSS_TEST_ENGINES_HOME=~/Library/Application\ Support/cross-app \
//!     cargo test -p cross-bottle --test real_wine -- --ignored --test-threads=1
//! ```
//!
//! `CROSS_TEST_ENGINES_HOME` points at a data directory that contains `engines/`.
//! Bottles are created in a throwaway directory; the engines are symlinked in.

use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use cross_bottle::exec::run_captured;
use cross_bottle::launch::{build_command, LaunchRequest};
use cross_bottle::manager::{BottleManager, CreateOptions};
use cross_bottle::processes::StopMode;
use cross_bottle::Runtime;
use cross_core::model::WindowsVersion;
use cross_core::paths::AppDirs;

const CMD: &str = r"C:\windows\system32\cmd.exe";

struct Env {
    _tmp: tempfile::TempDir,
    dirs: AppDirs,
    engine: String,
}

fn env() -> Env {
    let source = std::env::var_os("CROSS_TEST_ENGINES_HOME")
        .map(PathBuf::from)
        .expect("set CROSS_TEST_ENGINES_HOME to a data dir containing engines/");
    let tmp = tempfile::tempdir().unwrap();
    let dirs = AppDirs::with_root(tmp.path());
    symlink(source.join("engines"), dirs.engines_dir()).unwrap();
    let engine = std::env::var("CROSS_TEST_ENGINE").unwrap_or_else(|_| "wine-devel-11.18".into());
    Env {
        _tmp: tmp,
        dirs,
        engine,
    }
}

fn create(env: &Env, id: &str, windows: WindowsVersion) {
    BottleManager::new(&env.dirs)
        .create(&CreateOptions {
            id: id.into(),
            name: None,
            engine: env.engine.clone(),
            windows,
        })
        .unwrap();
}

/// Run `cmd /c <script>` in the bottle and return (exit code, stdout).
fn cmd_c(rt: &Runtime, script: &str) -> (Option<i32>, String) {
    let request = LaunchRequest {
        target: CMD.into(),
        args: vec!["/c".into(), script.into()],
        ..Default::default()
    };
    let cmd = build_command(rt, &request).unwrap();
    let out = run_captured(cmd, "cmd /c", Duration::from_secs(120)).unwrap();
    (out.status.code(), out.stdout)
}

#[test]
#[ignore = "needs a real Wine engine"]
fn bottle_lifecycle() {
    let env = env();
    let manager = BottleManager::new(&env.dirs);

    // create: a real prefix with the requested Windows version.
    create(&env, "demo", WindowsVersion::Win10);
    assert!(env.dirs.bottle_prefix("demo").join("system.reg").is_file());
    assert!(env.dirs.bottle_prefix("demo").join("drive_c").is_dir());
    let (_, rt) = Runtime::open(&env.dirs, "demo").unwrap();
    let version = rt
        .reg_get(r"HKCU\Software\Wine", "Version")
        .unwrap()
        .unwrap();
    assert_eq!(version.data, "win10");

    // run: output, exit code and the reported Windows version.
    let (code, out) = cmd_c(&rt, "ver");
    assert_eq!(code, Some(0));
    assert!(out.contains("10.0."), "{out}");
    assert_eq!(cmd_c(&rt, "exit 7").0, Some(7));

    // registry and presets.
    rt.reg_set(
        r"HKCU\Software\Cross",
        "Answer",
        cross_bottle::registry::RegType::Dword,
        "42",
    )
    .unwrap();
    assert_eq!(
        rt.reg_get(r"HKCU\Software\Cross", "Answer")
            .unwrap()
            .unwrap()
            .as_u64(),
        Some(42)
    );
    rt.reg_delete(r"HKCU\Software\Cross", "Answer").unwrap();
    assert_eq!(rt.reg_get(r"HKCU\Software\Cross", "Answer").unwrap(), None);
    rt.reg_delete(r"HKCU\Software\Cross", "Answer").unwrap(); // idempotent

    rt.set_windows_version(WindowsVersion::Win7).unwrap();
    assert!(cmd_c(&rt, "ver").1.contains("6.1."));
    rt.set_dpi(144).unwrap();
    assert_eq!(
        rt.reg_get(r"HKCU\Control Panel\Desktop", "LogPixels")
            .unwrap()
            .unwrap()
            .as_u64(),
        Some(144)
    );
    rt.enable_virtual_desktop(1280, 720).unwrap();
    assert_eq!(
        rt.reg_get(r"HKCU\Software\Wine\Explorer\Desktops", "Default")
            .unwrap()
            .unwrap()
            .data,
        "1280x720"
    );
    rt.disable_virtual_desktop().unwrap();
    assert_eq!(
        rt.reg_get(r"HKCU\Software\Wine\Explorer", "Desktop")
            .unwrap(),
        None
    );

    // clone straight after a run (the wineserver is still winding down), then rename.
    manager.clone_bottle("demo", "copy", Some("Copy")).unwrap();
    let (_, copy_rt) = Runtime::open(&env.dirs, "copy").unwrap();
    assert_eq!(
        copy_rt
            .reg_get(r"HKCU\Software\Wine", "Version")
            .unwrap()
            .unwrap()
            .data,
        "win7"
    );
    manager.rename("copy", "renamed").unwrap();
    let (_, renamed_rt) = Runtime::open(&env.dirs, "renamed").unwrap();
    assert_eq!(cmd_c(&renamed_rt, "exit 3").0, Some(3));

    // delete.
    manager.delete("renamed").unwrap();
    manager.delete("demo").unwrap();
    assert!(manager.list().unwrap().is_empty());
    assert!(!env.dirs.bottle_dir("demo").exists());
}

#[test]
#[ignore = "needs a real Wine engine"]
fn processes_can_be_listed_and_stopped() {
    let env = env();
    create(&env, "procs", WindowsVersion::Win11);
    let (_, rt) = Runtime::open(&env.dirs, "procs").unwrap();
    assert!(!rt.is_running().unwrap());
    assert!(rt.processes().unwrap().is_empty());

    // A long-running windowless program.
    let request = LaunchRequest {
        target: CMD.into(),
        args: vec!["/c".into(), "ping -n 120 127.0.0.1 > nul".into()],
        ..Default::default()
    };
    let mut child = build_command(&rt, &request)
        .unwrap()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(60);
    let names = loop {
        let names: Vec<String> = rt
            .processes()
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        if names.iter().any(|n| n == "ping.exe") || Instant::now() > deadline {
            break names;
        }
        std::thread::sleep(Duration::from_millis(500));
    };
    assert!(names.iter().any(|n| n == "ping.exe"), "{names:?}");
    assert!(rt.is_running().unwrap());

    // Guards: a running bottle cannot be cloned or renamed.
    let manager = BottleManager::new(&env.dirs);
    assert!(matches!(
        manager.clone_bottle("procs", "x", None).unwrap_err(),
        cross_bottle::Error::BottleRunning { .. }
    ));

    // `wineboot -e` only asks windowed programs to quit; a console `ping` survives it.
    let graceful = rt.stop(StopMode::Graceful).unwrap();
    assert!(graceful.was_running);

    let forced = rt.stop(StopMode::Force).unwrap();
    assert_eq!(forced.remaining, 0);
    assert!(!rt.is_running().unwrap());
    assert!(rt.processes().unwrap().is_empty());
    let _ = child.wait();

    // Stopping a stopped bottle is a no-op.
    assert!(!rt.stop(StopMode::Graceful).unwrap().was_running);
    manager.delete("procs").unwrap();
}
