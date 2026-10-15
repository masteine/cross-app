//! Translation checks against a real installed engine. `#[ignore]`d: needs one.
//!
//! ```text
//! CROSS_TEST_ENGINES_HOME=~/Library/Application\ Support/cross-app \
//!     cargo test -p cross-engine --test real_engine -- --ignored
//! ```

use std::path::PathBuf;

use cross_core::model::Translation;
use cross_core::paths::AppDirs;
use cross_engine::macho::{self, Arch};
use cross_engine::manager::EngineManager;
use cross_engine::policy::Host;
use cross_engine::Error;

#[test]
#[ignore]
fn gcenx_wine_is_x86_64_and_needs_rosetta() {
    let home = std::env::var_os("CROSS_TEST_ENGINES_HOME")
        .map(PathBuf::from)
        .expect("set CROSS_TEST_ENGINES_HOME to a data dir containing engines/");
    let id = std::env::var("CROSS_TEST_ENGINE").unwrap_or_else(|_| "wine-devel-11.18".into());
    let dirs = AppDirs::with_root(home);
    let manager = EngineManager::new(&dirs);
    let engine = manager.get(&id).unwrap().expect("engine installed");
    assert_eq!(engine.translation, Translation::Rosetta);

    let wine = manager.wine_path(&engine);
    assert_eq!(macho::architectures(&wine), Some(vec![Arch::X86_64]));
    assert!(macho::needs_rosetta(&wine));

    let mut host = Host::detect();
    if host.rosetta_installed {
        manager.check_runnable(&engine).unwrap();
    }
    host.rosetta_installed = false;
    let err = EngineManager::new(&dirs)
        .with_host(host)
        .check_runnable(&engine)
        .unwrap_err();
    assert!(matches!(err, Error::RosettaRequired { .. }), "{err}");
}
