//! Reads real CrossOver bottles (read-only). `#[ignore]`d: needs a Mac that has
//! (or had) CrossOver.
//!
//! ```text
//! CROSS_TEST_CROSSOVER_BOTTLES=~/Library/Application\ Support/CrossOver/Bottles \
//!     cargo test -p cross-engine --test real_crossover -- --ignored
//! ```

use std::path::PathBuf;

use cross_engine::crossover::{list_bottles, PrefixArch};

#[test]
#[ignore]
fn real_bottles_are_listed_with_their_settings() {
    let dir = std::env::var_os("CROSS_TEST_CROSSOVER_BOTTLES")
        .map(PathBuf::from)
        .expect("set CROSS_TEST_CROSSOVER_BOTTLES to a CrossOver Bottles directory");
    let bottles = list_bottles(&[dir]);
    assert!(!bottles.is_empty(), "no bottles found");
    for bottle in &bottles {
        println!(
            "{}: arch={:?} template={:?} crossover={:?}",
            bottle.name, bottle.arch, bottle.template, bottle.crossover_version
        );
        assert!(bottle.arch.is_some(), "{bottle:?}");
        assert!(bottle.template.is_some(), "{bottle:?}");
        assert!(bottle.crossover_version.is_some(), "{bottle:?}");
        if bottle
            .template
            .as_deref()
            .is_some_and(|t| t.ends_with("_64"))
        {
            assert_eq!(bottle.arch, Some(PrefixArch::Win64), "{bottle:?}");
        }
    }
}

/// `CROSS_TEST_CROSSOVER_APP=~/Applications/CrossOver.app cargo test -p cross-engine
/// --test real_crossover -- --ignored`: checks the documented bundle layout.
#[test]
#[ignore]
fn real_app_matches_the_documented_layout() {
    use cross_engine::crossover::CrossOverApp;
    use cross_engine::macho::Arch;

    let app = std::env::var_os("CROSS_TEST_CROSSOVER_APP")
        .map(PathBuf::from)
        .expect("set CROSS_TEST_CROSSOVER_APP to a CrossOver.app");
    let found = CrossOverApp::probe(&app).expect("bin/wine under Contents/SharedSupport/CrossOver");
    let loader = found
        .loader_architectures()
        .expect("bin/wineloader is Mach-O");
    println!(
        "CrossOver {:?} at {}: wineloader {:?}",
        found.version,
        found.app.display(),
        loader
    );
    assert!(found.version.is_some());
    assert!(loader.contains(&Arch::X86_64) || loader.contains(&Arch::Arm64));
}
