//! The verb catalog: format, parsing and validation.
//!
//! The catalog is embedded in the binary; `<data dir>/verbs.toml` replaces it
//! when present, so verbs can be updated without a new release.

use std::collections::{BTreeMap, HashSet};
use std::fs;

use cross_core::paths::AppDirs;
use serde::Deserialize;

use crate::{Error, Result};

const BUILTIN: &str = include_str!("../verbs.toml");
const SCHEMA_VERSION: u32 = 1;

/// DLL override modes Wine understands (`""` would disable a DLL; not offered).
const OVERRIDE_MODES: [&str; 4] = ["native", "builtin", "native,builtin", "builtin,native"];

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Download {
    /// Name used by `run` steps to refer to this file.
    pub id: String,
    /// File name the installer is run under.
    pub file: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
}

/// Where a `copy` step writes, relative to `drive_c/windows`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dest {
    /// 64-bit system DLLs.
    System32,
    /// 32-bit system DLLs.
    Syswow64,
    Fonts,
    Windows,
}

impl Dest {
    pub fn relative_dir(self) -> &'static str {
        match self {
            Dest::System32 => "drive_c/windows/system32",
            Dest::Syswow64 => "drive_c/windows/syswow64",
            Dest::Fonts => "drive_c/windows/Fonts",
            Dest::Windows => "drive_c/windows",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Step {
    /// Run a downloaded installer through Wine.
    Run {
        download: String,
        #[serde(default)]
        args: Vec<String>,
        /// Windows exit codes that count as success. Unix keeps only the low 8 bits of
        /// an exit status, so a code is matched modulo 256 (3010 arrives as 194).
        #[serde(default = "default_ok_exit")]
        ok_exit: Vec<i32>,
    },
    /// Copy files into the prefix. `from` is `component:<kind>:<subdir>`, the
    /// folder of an engine component (e.g. DXVK's `x86_64-windows`).
    Copy {
        from: String,
        to: Dest,
        /// File names or `*.ext` patterns.
        files: Vec<String>,
        /// Turn Wine builtin DLLs into native ones while copying (see
        /// `install::make_native`); Wine refuses to load a builtin-marked DLL
        /// from the prefix as native.
        #[serde(default)]
        native: bool,
    },
    /// Set DLL overrides.
    Override { dlls: Vec<String>, mode: String },
    /// Write a registry value.
    Registry {
        key: String,
        name: String,
        #[serde(rename = "type", default = "default_reg_type")]
        kind: String,
        data: String,
    },
}

fn default_ok_exit() -> Vec<i32> {
    vec![0]
}

fn default_reg_type() -> String {
    "sz".to_owned()
}

/// A `component:<kind>:<subdir>` source, parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentRef<'a> {
    pub kind: &'a str,
    pub subdir: &'a str,
}

pub fn parse_component_ref(from: &str) -> Option<ComponentRef<'_>> {
    let rest = from.strip_prefix("component:")?;
    let (kind, subdir) = rest.split_once(':')?;
    let safe = |s: &str| {
        !s.is_empty()
            && s.split('/')
                .all(|part| !part.is_empty() && part != ".." && part != ".")
    };
    (safe(kind) && !kind.contains('/') && safe(subdir)).then_some(ComponentRef { kind, subdir })
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Verb {
    pub title: String,
    /// Verbs installed first.
    #[serde(default)]
    pub requires: Vec<String>,
    /// Verbs that must not be installed alongside this one.
    #[serde(default)]
    pub conflicts: Vec<String>,
    #[serde(default)]
    pub downloads: Vec<Download>,
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VerbCatalog {
    pub schema: u32,
    pub verbs: BTreeMap<String, Verb>,
}

impl VerbCatalog {
    pub fn builtin() -> Self {
        Self::parse(BUILTIN).expect("embedded verbs.toml is valid (checked by a unit test)")
    }

    /// `<data dir>/verbs.toml` if it exists, otherwise the built-in catalog.
    pub fn load(dirs: &AppDirs) -> Result<Self> {
        let path = dirs.root().join("verbs.toml");
        match fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text)
                .map_err(|e| Error::InvalidCatalog(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::builtin()),
            Err(e) => Err(Error::io(path, e)),
        }
    }

    pub fn parse(text: &str) -> Result<Self> {
        let catalog: VerbCatalog = toml::from_str(text)
            .map_err(|e| Error::InvalidCatalog(e.to_string().trim().to_owned()))?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn get(&self, verb: &str) -> Option<&Verb> {
        self.verbs.get(verb)
    }

    fn validate(&self) -> Result<()> {
        let bad = |msg: String| Err(Error::InvalidCatalog(msg));
        if self.schema != SCHEMA_VERSION {
            return bad(format!(
                "unsupported schema {} (expected {SCHEMA_VERSION})",
                self.schema
            ));
        }
        for (name, verb) in &self.verbs {
            validate_verb_name(name)?;
            let mut ids = HashSet::new();
            for d in &verb.downloads {
                if !ids.insert(d.id.as_str()) {
                    return bad(format!("{name}: duplicate download id {:?}", d.id));
                }
                if !d.url.starts_with("https://") {
                    return bad(format!("{name}/{}: URL must use https", d.id));
                }
                let hex = d.sha256.len() == 64
                    && d.sha256
                        .bytes()
                        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
                if !hex {
                    return bad(format!(
                        "{name}/{}: sha256 must be 64 lowercase hex digits",
                        d.id
                    ));
                }
                if d.file.is_empty() || d.file.contains(['/', '\\']) || d.file.starts_with('.') {
                    return bad(format!("{name}/{}: file must be a plain file name", d.id));
                }
            }
            if verb.steps.is_empty() {
                return bad(format!("{name}: no steps"));
            }
            for dep in verb.requires.iter().chain(&verb.conflicts) {
                validate_verb_name(dep)?;
            }
            for req in &verb.requires {
                if !self.verbs.contains_key(req) {
                    return bad(format!("{name}: requires unknown verb {req:?}"));
                }
            }
            for step in &verb.steps {
                match step {
                    Step::Run {
                        download, ok_exit, ..
                    } => {
                        if !ids.contains(download.as_str()) {
                            return bad(format!(
                                "{name}: run step uses unknown download {download:?}"
                            ));
                        }
                        if ok_exit.is_empty() {
                            return bad(format!(
                                "{name}: run step needs at least one ok_exit code"
                            ));
                        }
                    }
                    Step::Copy { from, files, .. } => {
                        if parse_component_ref(from).is_none() {
                            return bad(format!(
                                "{name}: copy source {from:?} must look like component:<kind>:<subdir>"
                            ));
                        }
                        if files.is_empty()
                            || files
                                .iter()
                                .any(|f| f.contains(['/', '\\']) || f.contains(".."))
                        {
                            return bad(format!(
                                "{name}: copy files must be plain names or *.ext patterns"
                            ));
                        }
                    }
                    Step::Override { dlls, mode } => {
                        if dlls.is_empty() {
                            return bad(format!("{name}: override step lists no DLLs"));
                        }
                        if !OVERRIDE_MODES.contains(&mode.as_str()) {
                            return bad(format!(
                                "{name}: override mode {mode:?} must be one of {}",
                                OVERRIDE_MODES.join(", ")
                            ));
                        }
                    }
                    Step::Registry {
                        key, name: value, ..
                    } => {
                        if key.is_empty() || value.is_empty() {
                            return bad(format!("{name}: registry step needs a key and a name"));
                        }
                    }
                }
            }
        }
        // Reject dependency cycles.
        for name in self.verbs.keys() {
            let mut path = Vec::new();
            self.check_cycle(name, &mut path)?;
        }
        Ok(())
    }

    fn check_cycle<'a>(&'a self, name: &'a str, path: &mut Vec<&'a str>) -> Result<()> {
        if path.contains(&name) {
            return Err(Error::InvalidCatalog(format!(
                "dependency cycle: {} -> {name}",
                path.join(" -> ")
            )));
        }
        path.push(name);
        if let Some(verb) = self.verbs.get(name) {
            for req in &verb.requires {
                self.check_cycle(req, path)?;
            }
        }
        path.pop();
        Ok(())
    }

    /// `verbs` plus everything they require, dependencies first, no duplicates.
    pub fn resolve_order<'a>(&'a self, verbs: &'a [String]) -> Vec<&'a str> {
        fn visit<'a>(cat: &'a VerbCatalog, name: &'a str, out: &mut Vec<&'a str>) {
            if out.contains(&name) {
                return;
            }
            if let Some(verb) = cat.verbs.get(name) {
                for req in &verb.requires {
                    visit(cat, req, out);
                }
            }
            out.push(name);
        }
        let mut out = Vec::new();
        for v in verbs {
            visit(self, v, &mut out);
        }
        out
    }
}

/// Verb names are passed to other programs (winetricks), so keep them boring.
pub fn validate_verb_name(name: &str) -> Result<()> {
    let ok_char =
        |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.');
    let starts_ok = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if name.len() <= 64 && starts_ok && name.chars().all(ok_char) {
        Ok(())
    } else {
        Err(Error::InvalidVerbName(name.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_catalog_is_valid() {
        let catalog = VerbCatalog::builtin();
        let vcrun = catalog.get("vcrun2022").unwrap();
        assert_eq!(vcrun.downloads.len(), 2);
        assert!(catalog.get("dxvk").unwrap().downloads.is_empty());
    }

    fn with(text: &str) -> Result<VerbCatalog> {
        VerbCatalog::parse(&format!("schema = 1\n{text}"))
    }

    const GOOD_DL: &str = r#"
[verbs.a]
title = "A"
[[verbs.a.downloads]]
id = "d"
file = "a.exe"
url = "https://example.com/a.exe"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
[[verbs.a.steps]]
kind = "run"
download = "d"
"#;

    #[test]
    fn minimal_verb_parses_with_defaults() {
        let cat = with(GOOD_DL).unwrap();
        let Step::Run { ok_exit, args, .. } = &cat.get("a").unwrap().steps[0] else {
            panic!("expected a run step");
        };
        assert_eq!(ok_exit, &[0]);
        assert!(args.is_empty());
    }

    #[test]
    fn rejects_bad_downloads_and_steps() {
        let cases = [
            (GOOD_DL.replace("https://", "http://"), "https"),
            (GOOD_DL.replace("0000", "ABCD"), "sha256"),
            (GOOD_DL.replace("a.exe\"\nurl", "../a.exe\"\nurl"), "plain file name"),
            (GOOD_DL.replace("download = \"d\"", "download = \"zzz\""), "unknown download"),
            (GOOD_DL.replace("[[verbs.a.downloads]]", "[[verbs.a.downloads]]\nid = \"d\"\nfile=\"x\"\nurl=\"https://x\"\nsha256=\"0000000000000000000000000000000000000000000000000000000000000000\"\n[[verbs.a.downloads]]"), "duplicate"),
        ];
        for (text, needle) in cases {
            let err = with(&text).unwrap_err().to_string();
            assert!(err.contains(needle), "{needle}: {err}");
        }
    }

    #[test]
    fn rejects_bad_copy_override_and_cycles() {
        let copy = |from: &str, files: &str| {
            with(&format!(
                "[verbs.a]\ntitle=\"A\"\n[[verbs.a.steps]]\nkind=\"copy\"\nfrom=\"{from}\"\nto=\"system32\"\nfiles={files}\n"
            ))
        };
        assert!(copy("component:dxvk:x64", r#"["*.dll"]"#).is_ok());
        assert!(copy("/etc", r#"["*.dll"]"#).is_err());
        assert!(copy("component:dxvk:../x", r#"["*.dll"]"#).is_err());
        assert!(copy("component:dxvk:x64", r#"["../x.dll"]"#).is_err());
        assert!(copy("component:dxvk:x64", "[]").is_err());

        let ovr = |mode: &str| {
            with(&format!(
                "[verbs.a]\ntitle=\"A\"\n[[verbs.a.steps]]\nkind=\"override\"\ndlls=[\"d3d11\"]\nmode=\"{mode}\"\n"
            ))
        };
        assert!(ovr("native,builtin").is_ok());
        assert!(ovr("whatever").is_err());

        let cycle = "[verbs.a]\ntitle=\"A\"\nrequires=[\"b\"]\n[[verbs.a.steps]]\nkind=\"override\"\ndlls=[\"x\"]\nmode=\"native\"\n\
                     [verbs.b]\ntitle=\"B\"\nrequires=[\"a\"]\n[[verbs.b.steps]]\nkind=\"override\"\ndlls=[\"x\"]\nmode=\"native\"\n";
        assert!(with(cycle).unwrap_err().to_string().contains("cycle"));
        let dangling = "[verbs.a]\ntitle=\"A\"\nrequires=[\"nope\"]\n[[verbs.a.steps]]\nkind=\"override\"\ndlls=[\"x\"]\nmode=\"native\"\n";
        assert!(with(dangling)
            .unwrap_err()
            .to_string()
            .contains("unknown verb"));
    }

    #[test]
    fn requires_come_first_without_duplicates() {
        let step = "[[verbs.X.steps]]\nkind=\"override\"\ndlls=[\"x\"]\nmode=\"native\"\n";
        let text = format!(
            "[verbs.a]\ntitle=\"A\"\nrequires=[\"b\"]\n{}\n[verbs.b]\ntitle=\"B\"\nrequires=[\"c\"]\n{}\n[verbs.c]\ntitle=\"C\"\n{}",
            step.replace('X', "a"),
            step.replace('X', "b"),
            step.replace('X', "c"),
        );
        let cat = with(&text).unwrap();
        let wanted = vec!["a".to_owned(), "c".to_owned()];
        assert_eq!(cat.resolve_order(&wanted), ["c", "b", "a"]);
    }

    #[test]
    fn component_refs() {
        let r = parse_component_ref("component:dxvk:x86_64-windows").unwrap();
        assert_eq!((r.kind, r.subdir), ("dxvk", "x86_64-windows"));
        assert!(parse_component_ref("component:dxvk").is_none());
        assert!(parse_component_ref("file:dxvk:x").is_none());
        assert!(parse_component_ref("component:dxvk:a/../b").is_none());
    }

    #[test]
    fn verb_names_are_validated() {
        for ok in ["vcrun2022", "d3dcompiler_47", "dotnet48", "a.b-c"] {
            assert!(validate_verb_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-q", ".x", "A", "a b", "a/b", "a;rm"] {
            assert!(validate_verb_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn local_catalog_overrides_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());
        assert_eq!(VerbCatalog::load(&dirs).unwrap(), VerbCatalog::builtin());
        fs::write(dirs.root().join("verbs.toml"), "schema = 1\nverbs = {}\n").unwrap();
        assert!(VerbCatalog::load(&dirs).unwrap().verbs.is_empty());
        fs::write(dirs.root().join("verbs.toml"), "schema = [").unwrap();
        assert!(VerbCatalog::load(&dirs).is_err());
    }
}
