//! `steam-patches.toml`: the pinned Steam installer, Steam launch flags and
//! registry fixes, shipped and updated independently of cross-app releases.
//!
//! Because an updated file arrives over the network, what it may do is limited:
//! registry writes only under Wine's and Valve's own keys, launch arguments must
//! be plain `-flags`, and environment variables that would redirect the prefix or
//! inject libraries are refused.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use cross_bottle::registry::RegType;
use cross_bottle::Runtime;
use cross_core::model::LaunchProfile;
use cross_core::paths::AppDirs;
use serde::Deserialize;

use crate::{Error, Result};

const BUILTIN: &str = include_str!("../steam-patches.toml");
const SCHEMA_VERSION: u32 = 1;
const FILE_NAME: &str = "steam-patches.toml";
const MAX_DOWNLOAD: u64 = 256 * 1024;

/// Registry keys a patch may write under (compared case-insensitively).
const ALLOWED_KEY_PREFIXES: [&str; 4] = [
    r"hkcu\software\wine\",
    r"hkey_current_user\software\wine\",
    r"hkcu\software\valve\",
    r"hkey_current_user\software\valve\",
];
/// Environment variables a patch may not set.
const FORBIDDEN_ENV: [&str; 5] = ["WINEPREFIX", "WINEARCH", "WINESERVER", "WINE", "PATH"];
const OVERRIDE_MODES: [&str; 5] = ["", "native", "builtin", "native,builtin", "builtin,native"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InstallerPatch {
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct LaunchPatch {
    /// Arguments for `steam.exe`.
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// DLL -> mode; `""` disables the DLL.
    #[serde(default)]
    pub dll_overrides: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RegistryPatch {
    /// Why this exists (bug link); shown in logs.
    pub reason: String,
    pub key: String,
    pub name: String,
    #[serde(rename = "type", default = "default_type")]
    pub kind: String,
    pub data: String,
}

fn default_type() -> String {
    "sz".to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SteamPatches {
    pub schema: u32,
    /// Increases with every published change; updates never go backwards.
    pub revision: u32,
    pub installer: InstallerPatch,
    #[serde(default)]
    pub launch: LaunchPatch,
    #[serde(default)]
    pub registry: Vec<RegistryPatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Builtin,
    Local(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpdateOutcome {
    pub from: u32,
    pub to: u32,
    pub updated: bool,
}

impl SteamPatches {
    pub fn builtin() -> Self {
        Self::parse(BUILTIN).expect("embedded steam-patches.toml is valid (checked by a unit test)")
    }

    /// The downloaded copy in the data directory if present, else the built-in one.
    pub fn load(dirs: &AppDirs) -> Result<(Self, Source)> {
        let path = dirs.root().join(FILE_NAME);
        match fs::read_to_string(&path) {
            Ok(text) => {
                let patches = Self::parse(&text)
                    .map_err(|e| Error::InvalidPatches(format!("{}: {e}", path.display())))?;
                Ok((patches, Source::Local(path)))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok((Self::builtin(), Source::Builtin))
            }
            Err(e) => Err(Error::io(path, e)),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let patches: SteamPatches = toml::from_str(text)
            .map_err(|e| Error::InvalidPatches(e.to_string().trim().to_owned()))?;
        patches.validate()?;
        Ok(patches)
    }

    fn validate(&self) -> Result<()> {
        let bad = |msg: String| Err(Error::InvalidPatches(msg));
        if self.schema != SCHEMA_VERSION {
            return bad(format!(
                "unsupported schema {} (expected {SCHEMA_VERSION})",
                self.schema
            ));
        }
        if !self.installer.url.starts_with("https://") {
            return bad("installer URL must use https".into());
        }
        let hash = &self.installer.sha256;
        if hash.len() != 64 || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return bad("installer sha256 must be 64 lowercase hex digits".into());
        }
        for arg in self.installer.args.iter().chain(&self.launch.args) {
            let plain = (arg.starts_with('-') || arg.starts_with('/'))
                && arg.len() > 1
                && !arg
                    .chars()
                    .any(|c| c.is_whitespace() || matches!(c, '"' | '\'' | '\\' | ';'));
            if !plain {
                return bad(format!("argument {arg:?} must be a single -flag"));
            }
        }
        for key in self.launch.env.keys() {
            let name_ok = key
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
                && key
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
            if !name_ok || FORBIDDEN_ENV.contains(&key.as_str()) || key.starts_with("DYLD_") {
                return bad(format!("environment variable {key:?} is not allowed"));
            }
        }
        for (dll, mode) in &self.launch.dll_overrides {
            if dll.is_empty() || !OVERRIDE_MODES.contains(&mode.as_str()) {
                return bad(format!("invalid DLL override {dll}={mode}"));
            }
        }
        for patch in &self.registry {
            let key = patch.key.to_ascii_lowercase();
            if !ALLOWED_KEY_PREFIXES.iter().any(|p| key.starts_with(p)) {
                return bad(format!(
                    "registry key {:?} is outside HKCU\\Software\\Wine and HKCU\\Software\\Valve",
                    patch.key
                ));
            }
            if patch.name.is_empty() || RegType::parse(&patch.kind).is_none() {
                return bad(format!(
                    "registry patch {:?} needs a name and a valid type",
                    patch.reason
                ));
            }
        }
        Ok(())
    }

    /// Launch settings for `steam.exe` as a profile (args, env, DLL overrides).
    pub fn launch_profile(&self) -> LaunchProfile {
        LaunchProfile {
            args: self.launch.args.clone(),
            env: self.launch.env.clone(),
            dll_overrides: self.launch.dll_overrides.clone(),
            ..Default::default()
        }
    }

    /// Write every registry patch into the bottle.
    pub fn apply_registry(&self, runtime: &Runtime) -> Result<()> {
        for patch in &self.registry {
            tracing::info!(key = %patch.key, name = %patch.name, reason = %patch.reason, "applying Steam registry patch");
            let kind = RegType::parse(&patch.kind).unwrap_or(RegType::Sz);
            runtime.reg_set(&patch.key, &patch.name, kind, &patch.data)?;
        }
        Ok(())
    }
}

/// Install `text` as the local patch file if it is valid and newer than what is
/// in use (or `force` is set).
pub fn apply_update(dirs: &AppDirs, text: &str, force: bool) -> Result<UpdateOutcome> {
    let incoming = SteamPatches::parse(text)?;
    let (current, _) = SteamPatches::load(dirs)?;
    let outcome = UpdateOutcome {
        from: current.revision,
        to: incoming.revision,
        updated: force || incoming.revision > current.revision,
    };
    if outcome.updated {
        fs::create_dir_all(dirs.root()).map_err(|e| Error::io(dirs.root(), e))?;
        let path = dirs.root().join(FILE_NAME);
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(|e| Error::io(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| Error::io(&path, e))?;
        tracing::info!(
            from = outcome.from,
            to = outcome.to,
            "steam patches updated"
        );
    }
    Ok(outcome)
}

/// Fetch a patch file over https and apply it with [`apply_update`].
pub fn update_from(dirs: &AppDirs, url: &str, force: bool) -> Result<UpdateOutcome> {
    if !url.starts_with("https://") {
        return Err(Error::InvalidInput(format!(
            "patch URL must use https: {url}"
        )));
    }
    let bytes = cross_engine::download::fetch_bytes(url, MAX_DOWNLOAD)?
        .ok_or_else(|| Error::InvalidInput(format!("{url} does not exist (404)")))?;
    let text =
        String::from_utf8(bytes).map_err(|_| Error::InvalidPatches("not UTF-8 text".into()))?;
    apply_update(dirs, &text, force)
}

/// Forget the downloaded copy and go back to the built-in patches.
pub fn reset(dirs: &AppDirs) -> Result<()> {
    let path = dirs.root().join(FILE_NAME);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_patches_are_valid() {
        let p = SteamPatches::builtin();
        assert_eq!(p.installer.args, ["/S"]);
        assert!(p.launch.args.iter().all(|a| a.starts_with('-')));
        assert!(p
            .registry
            .iter()
            .any(|r| r.name == "gameoverlayrenderer" && r.data.is_empty()));
        assert_eq!(p.launch_profile().args, p.launch.args);
    }

    fn variant(edit: impl FnOnce(&mut String)) -> Result<SteamPatches> {
        let mut text = BUILTIN.to_owned();
        edit(&mut text);
        SteamPatches::parse(&text)
    }

    #[test]
    fn refuses_dangerous_or_malformed_patches() {
        type Edit = Box<dyn FnOnce(&mut String)>;
        let cases: Vec<(Edit, &str)> = vec![
            (
                Box::new(|t| *t = t.replace("https://cdn", "http://cdn")),
                "https",
            ),
            (
                Box::new(|t| *t = t.replace("7d3654531c", "ZZZZZZZZZZ")),
                "sha256",
            ),
            (
                Box::new(|t| *t = t.replace("\"-allosarches\"", "\"-x; rm -rf /\"")),
                "single -flag",
            ),
            (
                Box::new(|t| *t = t.replace("\"-allosarches\"", "\"C:\\\\evil.exe\"")),
                "single -flag",
            ),
            (
                Box::new(|t| {
                    t.push_str("\n[launch.env]\nDYLD_INSERT_LIBRARIES = \"/tmp/x.dylib\"\n")
                }),
                "not allowed",
            ),
            (
                Box::new(|t| t.push_str("\n[launch.env]\nWINEPREFIX = \"/\"\n")),
                "not allowed",
            ),
            (
                Box::new(|t| {
                    t.push_str("\n[[registry]]\nreason=\"x\"\nkey='HKLM\\System\\X'\nname=\"y\"\ndata=\"1\"\n")
                }),
                "outside",
            ),
            (
                Box::new(|t| {
                    t.push_str("\n[[registry]]\nreason=\"x\"\nkey='HKCU\\Software\\Wine\\X'\nname=\"y\"\ntype=\"weird\"\ndata=\"1\"\n")
                }),
                "valid type",
            ),
            (
                Box::new(|t| *t = t.replace("schema = 1", "schema = 9")),
                "schema",
            ),
        ];
        for (edit, needle) in cases {
            let err = variant(edit).unwrap_err().to_string();
            assert!(err.contains(needle), "{needle}: {err}");
        }
    }

    #[test]
    fn allows_safe_env_and_overrides() {
        let p = variant(|t| {
            t.push_str(
                "\n[launch.env]\nDXVK_ASYNC = \"1\"\n[launch.dll_overrides]\nlibglesv2 = \"\"\n",
            )
        })
        .unwrap();
        let profile = p.launch_profile();
        assert_eq!(profile.env["DXVK_ASYNC"], "1");
        assert_eq!(profile.launch_env()["WINEDLLOVERRIDES"], "libglesv2=");
    }

    #[test]
    fn updates_only_move_forward_unless_forced() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        assert_eq!(SteamPatches::load(&dirs).unwrap().1, Source::Builtin);

        let newer = BUILTIN.replace("revision = 1", "revision = 2");
        let out = apply_update(&dirs, &newer, false).unwrap();
        assert_eq!(
            out,
            UpdateOutcome {
                from: 1,
                to: 2,
                updated: true
            }
        );
        let (loaded, source) = SteamPatches::load(&dirs).unwrap();
        assert_eq!(loaded.revision, 2);
        assert!(matches!(source, Source::Local(_)));

        // Same or older revision: ignored.
        assert!(!apply_update(&dirs, &newer, false).unwrap().updated);
        assert!(!apply_update(&dirs, BUILTIN, false).unwrap().updated);
        assert_eq!(SteamPatches::load(&dirs).unwrap().0.revision, 2);
        // Forced downgrade.
        assert!(apply_update(&dirs, BUILTIN, true).unwrap().updated);
        assert_eq!(SteamPatches::load(&dirs).unwrap().0.revision, 1);

        // Invalid input never replaces the file.
        assert!(apply_update(&dirs, "schema = [", true).is_err());
        assert_eq!(SteamPatches::load(&dirs).unwrap().0.revision, 1);

        reset(&dirs).unwrap();
        assert_eq!(SteamPatches::load(&dirs).unwrap().1, Source::Builtin);
        reset(&dirs).unwrap();
    }

    #[test]
    fn update_requires_https() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        let err = update_from(&dirs, "http://example.com/p.toml", false).unwrap_err();
        assert!(err.to_string().contains("https"), "{err}");
    }
}
