//! CodeWeavers CrossOver as an external engine: finding the app and its bottles.
//!
//! The layout comes from CodeWeavers' documentation (see
//! `docs/adr/0003-post-rosetta.md`) and was checked, read-only, against a
//! CrossOver 26.2 trial found in `~/Applications` on the development Mac. Unit
//! tests use a fake bundle; no Windows program was started through the real app.
//!
//! - `CrossOver.app/Contents/SharedSupport/CrossOver/bin/wine` takes
//!   `--bottle <name>` (CrossOver User Guide, "Command Line").
//! - Bottle hooks run "in the Wine context" with `CX_ROOT` (the
//!   `.../SharedSupport/CrossOver` directory), `CX_BOTTLE` (bottle name) and
//!   `WINEPREFIX` set, and then "don't need to use the --bottle option"
//!   (Advanced CrossOver Mac Configuration).
//! - Bottles live in `~/Library/Application Support/CrossOver/Bottles`; the
//!   `BottleDir` preference (`defaults write com.codeweavers.CrossOver BottleDir`)
//!   and `CX_BOTTLE_PATH` (colon-separated) move them.
//! - In CrossOver 26.2, `bin/wine` is a Perl front end: the bottle is
//!   `--bottle`, else `$CX_BOTTLE`, else `default`; it is looked up as
//!   `<dir>/<name>/system.reg` in each `CX_BOTTLE_PATH` entry and `WINEPREFIX`
//!   is then set from it. `bin/wineloader` and `bin/wineserver` are x86_64-only
//!   Mach-O files there, so that CrossOver still needs Rosetta 2.
//! - Each bottle has a `cxbottle.conf` whose `[Bottle]` section holds lines like
//!   `"Template" = "win10_64"`, `"Version" = "26.2.0.39821"`, `"WineArch" = "win64"`.
//!   This format is not documented; it was read from a real CrossOver 26.2 bottle.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Bundle name looked for in the application folders.
pub const APP_NAME: &str = "CrossOver.app";
/// Overrides where `CrossOver.app` is looked for (used by tests and unusual installs).
pub const APP_ENV: &str = "CROSS_CROSSOVER_APP";
/// `CX_ROOT` inside the bundle.
const CX_ROOT_IN_APP: &str = "Contents/SharedSupport/CrossOver";
/// CrossOver's preferences domain (`defaults read com.codeweavers.CrossOver BottleDir`).
const DEFAULTS_DOMAIN: &str = "com.codeweavers.CrossOver";

/// An installed CrossOver.app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossOverApp {
    pub app: PathBuf,
    /// `CFBundleShortVersionString` from `Info.plist`.
    pub version: Option<String>,
}

impl CrossOverApp {
    /// `app` if it looks like CrossOver: a bundle with `bin/wine` under `CX_ROOT`.
    pub fn probe(app: &Path) -> Option<Self> {
        let found = Self {
            app: app.to_owned(),
            version: None,
        };
        if !found.wine().is_file() {
            return None;
        }
        Some(Self {
            version: bundle_version(app),
            ..found
        })
    }

    /// The first candidate that is a CrossOver bundle.
    pub fn find(candidates: &[PathBuf]) -> Option<Self> {
        candidates.iter().find_map(|c| Self::probe(c))
    }

    /// `$CROSS_CROSSOVER_APP` alone when set; otherwise `/Applications/CrossOver.app`
    /// and `~/Applications/CrossOver.app`.
    pub fn candidates() -> Vec<PathBuf> {
        if let Some(app) = std::env::var_os(APP_ENV).filter(|v| !v.is_empty()) {
            return vec![PathBuf::from(app)];
        }
        let mut list = vec![Path::new("/Applications").join(APP_NAME)];
        if let Some(home) = std::env::var_os("HOME") {
            list.push(Path::new(&home).join("Applications").join(APP_NAME));
        }
        list
    }

    /// `CX_ROOT`: `CrossOver.app/Contents/SharedSupport/CrossOver`.
    pub fn cx_root(&self) -> PathBuf {
        self.app.join(CX_ROOT_IN_APP)
    }

    /// CrossOver's `wine` front end.
    pub fn wine(&self) -> PathBuf {
        self.cx_root().join("bin").join("wine")
    }

    /// Architectures of the real Wine loader next to the front end
    /// (`bin/wineloader`); `None` if it is missing or not Mach-O.
    pub fn loader_architectures(&self) -> Option<Vec<crate::macho::Arch>> {
        crate::macho::architectures(&loader_of(&self.wine()))
    }
}

/// The Wine loader binary next to a `wine` front end (CrossOver: `bin/wineloader`).
pub fn loader_of(wine: &Path) -> PathBuf {
    wine.with_file_name("wineloader")
}

/// `CX_ROOT` for a CrossOver `wine` path (`<CX_ROOT>/bin/wine`).
pub fn cx_root_of(wine: &Path) -> Option<PathBuf> {
    let bin = wine.parent()?;
    (bin.file_name()? == "bin").then(|| bin.parent().map(Path::to_owned))?
}

/// `CFBundleShortVersionString` via `plutil`, which reads XML and binary plists.
fn bundle_version(app: &Path) -> Option<String> {
    let output = Command::new("plutil")
        .args(["-extract", "CFBundleShortVersionString", "raw", "-o", "-"])
        .arg(app.join("Contents/Info.plist"))
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (output.status.success() && !text.is_empty()).then(|| text.to_owned())
}

/// Bitness of a Wine prefix, from the `#arch=` line Wine writes into `system.reg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixArch {
    Win32,
    Win64,
}

impl PrefixArch {
    fn parse(text: &str) -> Option<Self> {
        match text {
            "win32" => Some(PrefixArch::Win32),
            "win64" => Some(PrefixArch::Win64),
            _ => None,
        }
    }
}

/// A CrossOver bottle (a Wine prefix in one of CrossOver's bottle directories).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossOverBottle {
    /// Directory name, which is what CrossOver calls the bottle.
    pub name: String,
    pub path: PathBuf,
    pub arch: Option<PrefixArch>,
    /// `Template` from `cxbottle.conf`, e.g. `win10_64`.
    pub template: Option<String>,
    /// `Version` from `cxbottle.conf`: the CrossOver version that last updated it.
    pub crossover_version: Option<String>,
}

impl CrossOverBottle {
    /// `dir` if it is a bottle: like CrossOver itself, require `system.reg`.
    pub fn probe(dir: &Path) -> Option<Self> {
        if !dir.join("system.reg").is_file() {
            return None;
        }
        let conf = fs::read_to_string(dir.join("cxbottle.conf")).unwrap_or_default();
        let value = |key: &str| conf_value(&conf, "Bottle", key);
        Some(Self {
            name: dir.file_name()?.to_str()?.to_owned(),
            path: dir.to_owned(),
            arch: value("WineArch")
                .as_deref()
                .and_then(PrefixArch::parse)
                .or_else(|| registry_arch(dir)),
            template: value("Template"),
            crossover_version: value("Version"),
        })
    }
}

/// `"Key" = "Value"` from `[section]` of a CrossOver `.conf` file; `;;` starts a comment.
fn conf_value(text: &str, section: &str, key: &str) -> Option<String> {
    let header = format!("[{section}]");
    let quoted = format!("\"{key}\"");
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == header;
            continue;
        }
        if !inside || line.starts_with(";;") {
            continue;
        }
        let Some(rest) = line.strip_prefix(&quoted) else {
            continue;
        };
        let value = rest.trim_start().strip_prefix('=')?.trim();
        let value = value.strip_prefix('"')?.strip_suffix('"')?;
        return (!value.is_empty()).then(|| value.to_owned());
    }
    None
}

/// The `#arch=` line Wine writes near the top of `system.reg`.
fn registry_arch(prefix: &Path) -> Option<PrefixArch> {
    let text = fs::read_to_string(prefix.join("system.reg")).ok()?;
    text.lines()
        .take(16)
        .find_map(|line| PrefixArch::parse(line.trim().strip_prefix("#arch=")?))
}

/// Where CrossOver keeps bottles: every `CX_BOTTLE_PATH` entry, the `BottleDir`
/// preference, and the default `~/Library/Application Support/CrossOver/Bottles`.
/// Duplicates are dropped; directories need not exist.
pub fn bottle_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(path) = std::env::var_os("CX_BOTTLE_PATH") {
        dirs.extend(std::env::split_paths(&path).filter(|p| !p.as_os_str().is_empty()));
    }
    if let Some(dir) = defaults_bottle_dir() {
        dirs.push(dir);
    }
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(Path::new(&home).join("Library/Application Support/CrossOver/Bottles"));
    }
    let mut unique = Vec::new();
    for dir in dirs {
        if !unique.contains(&dir) {
            unique.push(dir);
        }
    }
    unique
}

fn defaults_bottle_dir() -> Option<PathBuf> {
    let output = Command::new("defaults")
        .args(["read", DEFAULTS_DOMAIN, "BottleDir"])
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (output.status.success() && !text.is_empty()).then(|| PathBuf::from(text))
}

/// All bottles in `dirs`, sorted by name; the first directory wins on name clashes
/// (CrossOver creates bottles in the first one).
pub fn list_bottles(dirs: &[PathBuf]) -> Vec<CrossOverBottle> {
    let mut bottles: Vec<CrossOverBottle> = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        let mut found: Vec<_> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| CrossOverBottle::probe(&e.path()))
            .collect();
        found.retain(|b| !bottles.iter().any(|known| known.name == b.name));
        bottles.extend(found);
    }
    bottles.sort_by(|a, b| a.name.cmp(&b.name));
    bottles
}

/// The bottle called `name` in `dirs`.
pub fn find_bottle(dirs: &[PathBuf], name: &str) -> Option<CrossOverBottle> {
    list_bottles(dirs).into_iter().find(|b| b.name == name)
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    /// A fake `CrossOver.app` whose `bin/wine` is a recording shell script:
    /// each call appends `ARGS:`, `PREFIX:`, `CX:` (CX_ROOT|CX_BOTTLE) lines to
    /// `calls.log` next to it.
    pub fn fake_app(dir: &Path, version: &str) -> CrossOverApp {
        let app = dir.join(APP_NAME);
        let bin = app.join(CX_ROOT_IN_APP).join("bin");
        fs::create_dir_all(&bin).unwrap();
        let wine = bin.join("wine");
        let script = r#"#!/bin/sh
here="$(dirname "$0")"
echo "ARGS:$*" >> "$here/calls.log"
echo "PREFIX:$WINEPREFIX" >> "$here/calls.log"
echo "CX:$CX_ROOT|$CX_BOTTLE|$CX_BOTTLE_PATH" >> "$here/calls.log"
exit 0
"#;
        fs::write(&wine, script).unwrap();
        fs::set_permissions(&wine, fs::Permissions::from_mode(0o755)).unwrap();
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.codeweavers.CrossOver</string>
	<key>CFBundleShortVersionString</key>
	<string>{version}</string>
</dict>
</plist>
"#
        );
        fs::write(app.join("Contents/Info.plist"), plist).unwrap();
        CrossOverApp {
            app,
            version: Some(version.to_owned()),
        }
    }

    /// A fake bottle shaped like a real CrossOver 26.2 one: `drive_c`, a
    /// `system.reg` header as Wine writes it, and a `cxbottle.conf`.
    pub fn fake_bottle(dir: &Path, name: &str, arch: &str) -> PathBuf {
        let bottle = dir.join(name);
        fs::create_dir_all(bottle.join("drive_c/windows")).unwrap();
        fs::write(
            bottle.join("system.reg"),
            format!("WINE REGISTRY Version 2\n;; All keys relative to REGISTRY\\\\Machine\n\n#arch={arch}\n"),
        )
        .unwrap();
        let template = if arch == "win64" { "win10_64" } else { "winxp" };
        fs::write(
            bottle.join("cxbottle.conf"),
            format!(
                "[Bottle]\n\"MenuRoot\" = \"/Windows Applications\"\n\"Version\" = \"26.2.0.39821\"\n\
                 ;;\"Description\" = \"\"\n\"Template\" = \"{template}\"\n\"WineArch\" = \"{arch}\"\n\n\
                 [EnvironmentVariables]\n\"Template\" = \"not-this-one\"\n"
            ),
        )
        .unwrap();
        bottle
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{fake_app, fake_bottle};
    use super::*;

    #[test]
    fn detects_a_bundle_and_reads_its_version() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(CrossOverApp::probe(&tmp.path().join(APP_NAME)), None);
        let fake = fake_app(tmp.path(), "27.0.1");

        let found = CrossOverApp::find(&[tmp.path().join("nope.app"), fake.app.clone()]).unwrap();
        assert_eq!(found, fake);
        assert_eq!(found.version.as_deref(), Some("27.0.1"));
        assert_eq!(
            found.wine(),
            fake.app.join("Contents/SharedSupport/CrossOver/bin/wine")
        );
        assert_eq!(cx_root_of(&found.wine()), Some(found.cx_root()));
        assert_eq!(cx_root_of(Path::new("/x/wine")), None);
    }

    #[test]
    fn a_bundle_without_wine_is_not_crossover() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join(APP_NAME);
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        assert_eq!(CrossOverApp::probe(&app), None);
    }

    #[test]
    fn missing_version_is_tolerated() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = fake_app(tmp.path(), "1");
        fs::remove_file(fake.app.join("Contents/Info.plist")).unwrap();
        assert_eq!(CrossOverApp::probe(&fake.app).unwrap().version, None);
    }

    #[test]
    fn conf_values_are_read_from_the_right_section() {
        let conf = "[Bottle]\n;;\"Description\" = \"x\"\n\"Template\" = \"win10_64\"\n\
                    \"Empty\" = \"\"\n[Other]\n\"Template\" = \"no\"\n\"Only\" = \"here\"\n";
        assert_eq!(
            conf_value(conf, "Bottle", "Template").as_deref(),
            Some("win10_64")
        );
        assert_eq!(conf_value(conf, "Bottle", "Description"), None);
        assert_eq!(conf_value(conf, "Bottle", "Empty"), None);
        assert_eq!(conf_value(conf, "Bottle", "Only"), None);
        assert_eq!(conf_value(conf, "Other", "Only").as_deref(), Some("here"));
    }

    #[test]
    fn arch_falls_back_to_the_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let bottle = fake_bottle(tmp.path(), "Plain", "win32");
        fs::remove_file(bottle.join("cxbottle.conf")).unwrap();
        let found = CrossOverBottle::probe(&bottle).unwrap();
        assert_eq!(found.arch, Some(PrefixArch::Win32));
        assert_eq!(found.template, None);
    }

    #[test]
    fn lists_bottles_across_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let first = tmp.path().join("Bottles");
        let second = tmp.path().join("External");
        fake_bottle(&first, "Steam", "win64");
        fake_bottle(&first, "Old Office", "win32");
        fake_bottle(&second, "Steam", "win64"); // shadowed by the first directory
        fake_bottle(&second, "Games", "win64");
        fs::create_dir_all(first.join("not-a-bottle")).unwrap();
        fs::write(first.join("stray.txt"), "").unwrap();

        let bottles = list_bottles(&[first.clone(), second, tmp.path().join("missing")]);
        let names: Vec<_> = bottles.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["Games", "Old Office", "Steam"]);
        let steam = find_bottle(std::slice::from_ref(&first), "Steam").unwrap();
        assert_eq!(steam.path, first.join("Steam"));
        assert_eq!(steam.arch, Some(PrefixArch::Win64));
        assert_eq!(steam.template.as_deref(), Some("win10_64"));
        assert_eq!(steam.crossover_version.as_deref(), Some("26.2.0.39821"));
        assert_eq!(
            find_bottle(&[first], "Old Office").unwrap().arch,
            Some(PrefixArch::Win32)
        );
    }
}
