//! The GUI command layer against a real Wine engine (no window is opened).
//!
//! `#[ignore]`d like `cross-bottle`'s real-Wine tests; run with:
//!
//! ```text
//! CROSS_TEST_ENGINES_HOME=~/Library/Application\ Support/cross-app \
//!     cargo test -p cross-gui --test real_wine -- --ignored --test-threads=1
//! ```

use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use cross_core::model::WindowsVersion;
use cross_core::paths::AppDirs;
use cross_gui::api::{apps, bottles, engines, logs, system};

fn dirs() -> (tempfile::TempDir, AppDirs, String) {
    let source = std::env::var_os("CROSS_TEST_ENGINES_HOME")
        .map(PathBuf::from)
        .expect("set CROSS_TEST_ENGINES_HOME to a data dir containing engines/");
    let tmp = tempfile::tempdir().unwrap();
    let dirs = AppDirs::with_root(tmp.path().join("home"));
    std::fs::create_dir_all(dirs.root()).unwrap();
    symlink(source.join("engines"), dirs.engines_dir()).unwrap();
    let engine = std::env::var("CROSS_TEST_ENGINE").unwrap_or_else(|_| "wine-devel-11.18".into());
    (tmp, dirs, engine)
}

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[test]
#[ignore = "needs a real Wine engine (CROSS_TEST_ENGINES_HOME)"]
fn bottle_page_flow_on_real_wine() {
    let (tmp, dirs, engine) = dirs();

    let status = system::status(&dirs).unwrap();
    assert!(status.needs_setup);
    assert!(status.engines_installed.contains(&engine));
    let view = engines::list(&dirs).unwrap();
    assert!(view.engines.iter().any(|e| e.id == engine && e.installed));

    // Wizard: first bottle.
    let summary = bottles::create(
        &dirs,
        serde_json::from_value(serde_json::json!({
            "id": "gui",
            "name": "GUI test",
            "engine": engine,
            "windows": "win10",
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(summary.windows, WindowsVersion::Win10);
    assert!(!system::status(&dirs).unwrap().needs_setup);

    // Drag-and-drop: a batch file that keeps the bottle busy for a few seconds.
    let bat = tmp.path().join("busy.bat");
    std::fs::write(
        &bat,
        "@echo off\r\necho hello from the gui test\r\nping -n 6 127.0.0.1 > nul\r\necho done\r\n",
    )
    .unwrap();
    let app = apps::add(&dirs, "gui", &bat, None).unwrap();
    assert_eq!(app.name, "busy");

    let mut spawned = apps::spawn_program(&dirs, "gui", &app.path).unwrap();
    wait_for("the bottle to run", Duration::from_secs(60), || {
        bottles::list(&dirs).unwrap()[0].running
    });
    let names: Vec<String> = bottles::processes(&dirs, "gui")
        .unwrap()
        .into_iter()
        .map(|p| p.name.to_lowercase())
        .collect();
    assert!(names.iter().any(|n| n == "cmd.exe"), "{names:?}");

    let status = spawned.child.wait().unwrap();
    assert!(status.success(), "{status}");

    // Logs page: the program's output is in the bottle's logs/.
    let files = logs::list(&dirs, Some("gui")).unwrap();
    assert!(files.iter().any(|f| f.name == "busy.log"), "{files:?}");
    let chunk = logs::read(&dirs, Some("gui"), "busy.log", None).unwrap();
    assert!(
        chunk.text.contains("hello from the gui test"),
        "{}",
        chunk.text
    );
    assert!(chunk.text.contains("done"), "{}", chunk.text);

    // Settings and stop.
    bottles::set_windows(&dirs, "gui", WindowsVersion::Win11).unwrap();
    let detail = bottles::detail(&dirs, "gui").unwrap();
    assert_eq!(detail.summary.windows, WindowsVersion::Win11);
    bottles::stop(&dirs, "gui", true).unwrap();
    wait_for("the bottle to stop", Duration::from_secs(30), || {
        !bottles::list(&dirs).unwrap()[0].running
    });
    bottles::delete(&dirs, "gui").unwrap();
    assert!(bottles::list(&dirs).unwrap().is_empty());
}
