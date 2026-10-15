//! The graphics backend switch against a real Wine engine.
//!
//! `#[ignore]`d: it needs an installed engine with the DXVK components and takes
//! about a minute. The engine is cloned (APFS, `cp -cR`) into a throwaway data
//! directory because the D3DMetal overlay is written next to it.
//!
//! ```text
//! CROSS_TEST_ENGINES_HOME=~/Library/Application\ Support/cross-app \
//!     cargo test -p cross-deps --test real_graphics -- --ignored
//! ```
//!
//! What it proves: a d3dmetal launch loads `dxgi.dll` from the overlay (here a
//! synthetic, non-working stand-in for Apple's forwarder, so the load fails on
//! purpose) while the plain engine loads Wine's own; a dxvk launch loads DXVK's
//! `d3d11.dll` from the prefix as native. It does not prove that D3DMetal or
//! DXVK render anything: there is no real GPTK here and no game runs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use cross_bottle::exec::run_captured;
use cross_bottle::graphics::{self, EffectiveBackend, GraphicsPlan};
use cross_bottle::launch::{build_command, LaunchRequest};
use cross_bottle::manager::{BottleManager, CreateOptions};
use cross_core::config::BottleStore;
use cross_core::model::{GraphicsBackend, LaunchProfile, WindowsVersion};
use cross_core::paths::AppDirs;
use cross_deps::install::{InstallOptions, Installer};
use cross_engine::gptk;
use cross_engine::manager::EngineManager;

const ENGINE: &str = "wine-devel-11.18";

/// `regsvr32 /s <dll>` with `+loaddll` tracing; returns the combined output.
fn load_trace(dirs: &AppDirs, plan: &GraphicsPlan, dll: &str) -> String {
    let mut profile = LaunchProfile::default();
    plan.apply_to(&mut profile);
    let request = LaunchRequest {
        target: "regsvr32".into(),
        args: vec!["/s".into(), dll.into()],
        profile,
        ..Default::default()
    };
    let mut cmd = build_command(&plan.runtime(dirs, "gfx"), &request).unwrap();
    cmd.env("WINEDEBUG", "+loaddll");
    let out = run_captured(cmd, "regsvr32", Duration::from_secs(120)).unwrap();
    format!("{}{}", out.stdout, out.stderr)
}

fn fake_gptk(root: &Path) -> PathBuf {
    let lib = root.join("Game_Porting_Toolkit_4.0/redist/lib");
    fs::create_dir_all(lib.join("external/D3DMetal.framework")).unwrap();
    fs::write(lib.join("external/libd3dshared.dylib"), "not a dylib").unwrap();
    fs::write(lib.join("external/D3DMetal.framework/D3DMetal"), "x").unwrap();
    fs::create_dir_all(lib.join("wine/x86_64-windows")).unwrap();
    for name in gptk::FORWARDERS {
        let mut dll = b"MZ".to_vec();
        dll.resize(0x40, 0);
        dll.extend_from_slice(b"Wine builtin DLL");
        dll.extend_from_slice(b"synthetic stand-in, not Apple's code");
        fs::write(lib.join("wine/x86_64-windows").join(name), dll).unwrap();
    }
    root.join("Game_Porting_Toolkit_4.0")
}

#[test]
#[ignore = "needs a real Wine engine with DXVK components"]
fn backend_switch_loads_the_right_dlls() {
    let source = PathBuf::from(
        std::env::var_os("CROSS_TEST_ENGINES_HOME").expect("set CROSS_TEST_ENGINES_HOME"),
    );
    let tmp = tempfile::tempdir().unwrap();
    let dirs = AppDirs::with_root(tmp.path().join("home"));
    fs::create_dir_all(dirs.engines_dir()).unwrap();
    let cloned = Command::new("cp")
        .arg("-cR")
        .arg(source.join("engines").join(ENGINE))
        .arg(dirs.engines_dir())
        .status()
        .unwrap();
    assert!(cloned.success());

    // D3DMetal: overlay from a synthetic GPTK (forced: upstream Wine).
    let gptk_dir = dirs.gptk_dir();
    gptk::import(&fake_gptk(tmp.path()), &gptk_dir, None).unwrap();
    let engines = EngineManager::new(&dirs);
    engines.enable_d3dmetal(ENGINE, &gptk_dir, true).unwrap();

    BottleManager::new(&dirs)
        .create(&CreateOptions {
            id: "gfx".into(),
            name: None,
            engine: ENGINE.into(),
            windows: WindowsVersion::Win10,
        })
        .unwrap();
    let store = BottleStore::new(&dirs);

    let plan = graphics::plan_for(&dirs, &store.load("gfx").unwrap(), None).unwrap();
    assert_eq!(plan.effective, EffectiveBackend::D3dMetal);
    let trace = load_trace(&dirs, &plan, "dxgi.dll");
    assert!(trace.contains("Failed to load DLL 'dxgi.dll'"), "{trace}");
    assert!(!trace.contains(r#"system32\\dxgi.dll" at"#), "{trace}");

    // Without the overlay the same launch loads Wine's own dxgi.
    engines.disable_d3dmetal(ENGINE).unwrap();
    let plan = graphics::plan_for(&dirs, &store.load("gfx").unwrap(), None).unwrap();
    assert_eq!(plan.effective, EffectiveBackend::Wined3d);
    let trace = load_trace(&dirs, &plan, "dxgi.dll");
    assert!(
        trace.contains(r#"system32\\dxgi.dll" at"#) && trace.contains(": builtin"),
        "{trace}"
    );

    // DXVK: the verb's DLLs load as native.
    Installer::new(&dirs)
        .unwrap()
        .install(
            "gfx",
            &["dxvk".into()],
            InstallOptions::default(),
            &mut |_| {},
        )
        .unwrap();
    let mut bottle = store.load("gfx").unwrap();
    bottle.backend = GraphicsBackend::Dxvk;
    store.save(&bottle).unwrap();
    let plan = graphics::plan_for(&dirs, &bottle, None).unwrap();
    assert_eq!(plan.effective, EffectiveBackend::Dxvk);
    let trace = load_trace(&dirs, &plan, "d3d11.dll");
    assert!(
        trace
            .lines()
            .any(|l| l.contains(r#"system32\\d3d11.dll" at"#) && l.ends_with(": native")),
        "{trace}"
    );

    let (_, rt) = cross_bottle::Runtime::open(&dirs, "gfx").unwrap();
    rt.stop(cross_bottle::processes::StopMode::Force).unwrap();
}
