//! The game profile format: one TOML file per game.
//!
//! ```toml
//! schema = 1
//! id = "hollow-knight"              # [a-z0-9-], also the file name
//! name = "Hollow Knight"
//! steam_appid = 367520              # optional
//! exe_names = ["hollow_knight.exe"] # weak match, case-insensitive
//! exe_sha256 = []                   # strong match, lowercase hex
//! rating = "green"                  # green | yellow | red
//! backend = "d3dmetal"              # optional: d3dmetal | dxvk
//! sync = "msync"                    # optional: none | esync | msync
//! deps = ["vcrun2022"]              # verbs for `cross deps install`
//! env = {}                          # extra environment
//! dll_overrides = {}                # DLL -> mode
//! args = []                         # plain -flags only
//! anticheat = "EasyAntiCheat"       # optional
//! known_bugs = ["..."]
//! notes = ["..."]
//!
//! [source]                          # where the settings come from
//! name = "..."
//! url = "https://..."
//! reported_rating = "works"
//!
//! [verified]                        # only after a real run on our stack
//! date = "2026-10-15"
//! engine = "wine-devel-11.18"
//! macos = "26.1"
//! chip = "M3"
//! ```
//!
//! Community and local files arrive from outside, so the same limits as for
//! `steam-patches.toml` apply: no variables that redirect the prefix, the
//! loader or the library path; arguments must be plain `-flags`.

use std::collections::BTreeMap;

use cross_core::model::{GraphicsBackend, LaunchProfile, SyncMode};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const SCHEMA_VERSION: u32 = 1;

/// Environment variables a profile may not set (plus anything `DYLD_*`/`LD_*`).
const FORBIDDEN_ENV: [&str; 10] = [
    "WINEPREFIX",
    "WINEARCH",
    "WINE",
    "WINESERVER",
    "WINELOADER",
    "WINEDLLPATH",
    "WINEDEBUG",
    "PATH",
    "HOME",
    "TMPDIR",
];
const OVERRIDE_MODES: [&str; 11] = [
    "",
    "d",
    "disabled",
    "n",
    "b",
    "n,b",
    "b,n",
    "native",
    "builtin",
    "native,builtin",
    "builtin,native",
];

/// Expected compatibility, shown as a coloured badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Rating {
    /// Starts without manual steps and plays well.
    Green,
    /// Plays with workarounds, or nobody has confirmed it yet.
    Yellow,
    /// Does not run (anti-cheat, DRM, crashes).
    Red,
}

impl Rating {
    pub fn as_str(self) -> &'static str {
        match self {
            Rating::Green => "green",
            Rating::Yellow => "yellow",
            Rating::Red => "red",
        }
    }
}

/// Where a profile's settings come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Source {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The rating as the source states it, in its own words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_rating: Option<String>,
    /// What the source says it was tested with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_on: Option<String>,
}

/// A run of the game with cross-app that confirmed the profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    pub date: String,
    pub engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub macos: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameProfile {
    pub schema: u32,
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_appid: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exe_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exe_sha256: Vec<String>,
    pub rating: Rating,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<GraphicsBackend>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dll_overrides: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anticheat: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_bugs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    pub source: Source,
    /// `None` until someone ran the game with cross-app and confirmed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified: Option<Verification>,
}

impl GameProfile {
    /// Parse and validate one profile file; `origin` names it in errors.
    pub fn parse(text: &str, origin: &str) -> Result<Self> {
        let profile: GameProfile =
            toml::from_str(text).map_err(|e| Error::invalid(origin, e.to_string()))?;
        profile.validate(origin)?;
        Ok(profile)
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn is_verified(&self) -> bool {
        self.verified.is_some()
    }

    /// Short text for a badge, e.g. `yellow (unverified)`.
    pub fn badge_text(&self) -> String {
        if self.is_verified() {
            self.rating.as_str().to_owned()
        } else {
            format!("{} (unverified)", self.rating.as_str())
        }
    }

    /// Whether `file_name` (an `.exe` name) is one of this game's.
    pub fn matches_exe_name(&self, file_name: &str) -> bool {
        self.exe_names
            .iter()
            .any(|n| n.eq_ignore_ascii_case(file_name))
    }

    /// The launch settings this profile describes (backend, env, DLL
    /// overrides, arguments, sync). Dependencies are installed separately.
    pub fn launch_profile(&self) -> LaunchProfile {
        let mut profile = LaunchProfile {
            args: self.args.clone(),
            backend: self.backend,
            env: self.env.clone(),
            dll_overrides: self.dll_overrides.clone(),
            ..Default::default()
        };
        profile.graphics.sync = self.sync;
        profile
    }

    pub fn validate(&self, origin: &str) -> Result<()> {
        let bad = |m: String| Err(Error::invalid(origin, m));
        if self.schema != SCHEMA_VERSION {
            return bad(format!(
                "schema {} is not supported (expected {SCHEMA_VERSION})",
                self.schema
            ));
        }
        if !is_slug(&self.id) {
            return bad(format!(
                "id {:?} must be 1-64 characters of [a-z0-9-]",
                self.id
            ));
        }
        if self.name.trim().is_empty() {
            return bad("name is empty".into());
        }
        if self.steam_appid.is_none() && self.exe_names.is_empty() && self.exe_sha256.is_empty() {
            return bad("needs steam_appid, exe_names or exe_sha256 to match a game".into());
        }
        for hash in &self.exe_sha256 {
            if hash.len() != 64 || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                return bad(format!("exe_sha256 {hash:?} is not lowercase hex SHA-256"));
            }
        }
        for name in &self.exe_names {
            if name.contains(['/', '\\']) || !name.to_ascii_lowercase().ends_with(".exe") {
                return bad(format!(
                    "exe_names entry {name:?} must be a bare .exe file name"
                ));
            }
        }
        for key in self.env.keys() {
            let upper = key.to_ascii_uppercase();
            let shape_ok = !key.is_empty()
                && key
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
            if !shape_ok
                || FORBIDDEN_ENV.contains(&upper.as_str())
                || upper.starts_with("DYLD_")
                || upper.starts_with("LD_")
            {
                return bad(format!("environment variable {key:?} is not allowed"));
            }
        }
        for (dll, mode) in &self.dll_overrides {
            if !is_name(dll) || !OVERRIDE_MODES.contains(&mode.as_str()) {
                return bad(format!("DLL override {dll}={mode} is not allowed"));
            }
        }
        for dep in &self.deps {
            if !is_name(dep) {
                return bad(format!("dependency {dep:?} is not a verb name"));
            }
        }
        for arg in &self.args {
            let plain = arg.starts_with('-')
                && arg.len() > 1
                && arg
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'=' | b'.'));
            if !plain {
                return bad(format!("argument {arg:?} must be a plain -flag"));
            }
        }
        if let Some(url) = &self.source.url {
            if !url.starts_with("https://") {
                return bad(format!("source url {url:?} must use https"));
            }
        }
        if self.source.name.trim().is_empty() {
            return bad("source.name is empty: say where the settings come from".into());
        }
        Ok(())
    }
}

fn is_slug(id: &str) -> bool {
    let first_ok = id
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    first_ok
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SAMPLE: &str = r#"
schema = 1
id = "sample-game"
name = "Sample Game"
steam_appid = 42
exe_names = ["Sample.exe"]
rating = "yellow"
backend = "dxvk"
sync = "msync"
deps = ["vcrun2022", "dxvk"]
env = { DXVK_ASYNC = "1" }
dll_overrides = { xinput1_3 = "native" }
args = ["-windowed"]
known_bugs = ["menus flicker"]

[source]
name = "test fixture"
url = "https://example.invalid/sample"
"#;

    #[test]
    fn parses_and_converts_to_a_launch_profile() {
        let p = GameProfile::parse(SAMPLE, "sample").unwrap();
        assert_eq!(p.rating, Rating::Yellow);
        assert!(!p.is_verified());
        assert_eq!(p.badge_text(), "yellow (unverified)");
        assert!(p.matches_exe_name("sample.EXE"));
        let lp = p.launch_profile();
        assert_eq!(lp.backend, Some(GraphicsBackend::Dxvk));
        assert_eq!(lp.graphics.sync, Some(SyncMode::Msync));
        assert_eq!(lp.env["DXVK_ASYNC"], "1");
        assert_eq!(lp.args, ["-windowed"]);
        // Round trip.
        assert_eq!(GameProfile::parse(&p.to_toml().unwrap(), "rt").unwrap(), p);
    }

    #[test]
    fn rejects_dangerous_or_malformed_profiles() {
        let cases = [
            ("schema = 1", "schema = 2"),
            (r#"id = "sample-game""#, r#"id = "../x""#),
            (
                r#"{ DXVK_ASYNC = "1" }"#,
                r#"{ DYLD_INSERT_LIBRARIES = "/x" }"#,
            ),
            (r#"{ DXVK_ASYNC = "1" }"#, r#"{ WINEPREFIX = "/x" }"#),
            (r#"{ DXVK_ASYNC = "1" }"#, r#"{ lower = "1" }"#),
            (r#"xinput1_3 = "native""#, r#"xinput1_3 = "evil""#),
            (r#"["-windowed"]"#, r#"["; rm -rf /"]"#),
            (r#"["vcrun2022", "dxvk"]"#, r#"["Bad Verb"]"#),
            (r#"["Sample.exe"]"#, r#"["C:\\x\\Sample.exe"]"#),
            (
                "https://example.invalid/sample",
                "http://example.invalid/sample",
            ),
            (r#"name = "test fixture""#, r#"name = " ""#),
            (r#"rating = "yellow""#, r#"rating = "purple""#),
        ];
        for (from, to) in cases {
            let text = SAMPLE.replacen(from, to, 1);
            assert_ne!(text, SAMPLE, "fixture did not contain {from:?}");
            let err = GameProfile::parse(&text, "case").unwrap_err();
            assert!(matches!(err, Error::InvalidProfile { .. }), "{to}: {err}");
        }
        let unmatched = SAMPLE
            .replace("steam_appid = 42\n", "")
            .replace("exe_names = [\"Sample.exe\"]\n", "");
        assert!(GameProfile::parse(&unmatched, "x").is_err());
    }
}
