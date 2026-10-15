//! The compatibility database: built-in profiles, a community bundle and local
//! overrides, merged by profile id (local wins over community, community over
//! built-in).
//!
//! On disk, under the data directory:
//!
//! ```text
//! compat/
//! ├── community/bundle.toml   # fetched by `cross compat sync`
//! └── local/<id>.toml         # your own profiles and overrides
//! ```
//!
//! A community repository keeps one reviewed file per game in `profiles/` and
//! publishes `bundle.toml` (all profiles plus a `revision`) built with
//! [`make_bundle`]; see `docs/compat.md`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use cross_core::model::{App, Bottle};
use cross_core::paths::AppDirs;
use serde::{Deserialize, Serialize};

use crate::profile::{GameProfile, SCHEMA_VERSION};
use crate::{Error, Result};

/// Directory under the data root.
pub const COMPAT_DIR: &str = "compat";
const BUNDLE_FILE: &str = "bundle.toml";
const MAX_BUNDLE: u64 = 4 * 1024 * 1024;

/// Profiles shipped with cross-app. All are unverified: written from the
/// sources named in each file, not from runs on our stack.
const BUILTIN: [(&str, &str); 10] = [
    (
        "age-of-empires-2-de",
        include_str!("../profiles/age-of-empires-2-de.toml"),
    ),
    (
        "baldurs-gate-3",
        include_str!("../profiles/baldurs-gate-3.toml"),
    ),
    (
        "cyberpunk-2077",
        include_str!("../profiles/cyberpunk-2077.toml"),
    ),
    ("elden-ring", include_str!("../profiles/elden-ring.toml")),
    (
        "hollow-knight",
        include_str!("../profiles/hollow-knight.toml"),
    ),
    ("pubg", include_str!("../profiles/pubg.toml")),
    ("skyrim-se", include_str!("../profiles/skyrim-se.toml")),
    (
        "stardew-valley",
        include_str!("../profiles/stardew-valley.toml"),
    ),
    ("terraria", include_str!("../profiles/terraria.toml")),
    (
        "the-witcher-3",
        include_str!("../profiles/the-witcher-3.toml"),
    ),
];

/// Where a profile in the database came from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Builtin,
    Community,
    Local,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub profile: GameProfile,
    pub layer: Layer,
}

/// How a game was recognised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchKind {
    SteamAppid,
    /// The executable's SHA-256 is listed: exactly this build.
    ExeSha256,
    /// Only the file name matched: likely, not certain.
    ExeName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match<'a> {
    pub entry: &'a Entry,
    pub kind: MatchKind,
}

/// The merged database.
#[derive(Debug, Clone, Default)]
pub struct CompatDb {
    entries: BTreeMap<String, Entry>,
    /// Files that could not be used, with the reason; they are skipped.
    pub problems: Vec<String>,
}

pub fn community_dir(dirs: &AppDirs) -> PathBuf {
    dirs.root().join(COMPAT_DIR).join("community")
}

pub fn local_dir(dirs: &AppDirs) -> PathBuf {
    dirs.root().join(COMPAT_DIR).join("local")
}

impl CompatDb {
    /// Only the profiles built into cross-app.
    pub fn builtin() -> Self {
        let mut db = Self::default();
        for (id, text) in BUILTIN {
            let profile = GameProfile::parse(text, &format!("built-in {id}"))
                .expect("built-in profiles are valid (checked by tests)");
            db.insert(profile, Layer::Builtin);
        }
        db
    }

    /// Built-in, community and local profiles. Broken community or local
    /// files do not fail the load; they are listed in [`CompatDb::problems`].
    pub fn load(dirs: &AppDirs) -> Result<Self> {
        let mut db = Self::builtin();
        let bundle_path = community_dir(dirs).join(BUNDLE_FILE);
        match fs::read_to_string(&bundle_path) {
            Ok(text) => match Bundle::parse(&text, &bundle_path.display().to_string()) {
                Ok(bundle) => {
                    for profile in bundle.profile {
                        db.insert(profile, Layer::Community);
                    }
                }
                Err(e) => db.problems.push(e.to_string()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(bundle_path, e)),
        }

        let local = local_dir(dirs);
        let mut files: Vec<PathBuf> = match fs::read_dir(&local) {
            Ok(entries) => entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                .collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(Error::io(local, e)),
        };
        files.sort();
        for path in files {
            let origin = path.display().to_string();
            let parsed = fs::read_to_string(&path)
                .map_err(|e| Error::io(&path, e))
                .and_then(|text| GameProfile::parse(&text, &origin));
            match parsed {
                Ok(profile) => db.insert(profile, Layer::Local),
                Err(e) => db.problems.push(e.to_string()),
            }
        }
        Ok(db)
    }

    fn insert(&mut self, profile: GameProfile, layer: Layer) {
        self.entries
            .insert(profile.id.clone(), Entry { profile, layer });
    }

    /// All profiles, sorted by id.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.values()
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    pub fn by_appid(&self, appid: u32) -> Option<&Entry> {
        self.entries
            .values()
            .find(|e| e.profile.steam_appid == Some(appid))
    }

    /// Recognise an executable on this Mac: by SHA-256 first, then by name.
    pub fn by_exe(&self, exe: &Path) -> Result<Option<Match<'_>>> {
        let hash = cross_engine::download::hash_file(exe)?;
        if let Some(entry) = self
            .entries
            .values()
            .find(|e| e.profile.exe_sha256.contains(&hash))
        {
            return Ok(Some(Match {
                entry,
                kind: MatchKind::ExeSha256,
            }));
        }
        let name = exe
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(self
            .entries
            .values()
            .find(|e| e.profile.matches_exe_name(&name))
            .map(|entry| Match {
                entry,
                kind: MatchKind::ExeName,
            }))
    }
}

/// A community bundle: every profile of a repository plus a revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bundle {
    pub schema: u32,
    pub revision: u32,
    #[serde(default)]
    pub profile: Vec<GameProfile>,
}

impl Bundle {
    pub fn parse(text: &str, origin: &str) -> Result<Self> {
        let bundle: Bundle =
            toml::from_str(text).map_err(|e| Error::invalid(origin, e.to_string()))?;
        if bundle.schema != SCHEMA_VERSION {
            return Err(Error::invalid(
                origin,
                format!("bundle schema {} is not supported", bundle.schema),
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for p in &bundle.profile {
            p.validate(&format!("{origin} ({})", p.id))?;
            if !seen.insert(p.id.as_str()) {
                return Err(Error::invalid(
                    origin,
                    format!("duplicate profile id {}", p.id),
                ));
            }
        }
        Ok(bundle)
    }
}

/// Build `bundle.toml` from a repository's `profiles/*.toml` files.
pub fn make_bundle(profiles_dir: &Path, revision: u32) -> Result<String> {
    let mut files: Vec<PathBuf> = fs::read_dir(profiles_dir)
        .map_err(|e| Error::io(profiles_dir, e))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    let mut profile = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let p = GameProfile::parse(&text, &path.display().to_string())?;
        if path.file_stem().is_some_and(|s| s != p.id.as_str()) {
            return Err(Error::invalid(
                path.display().to_string(),
                format!("file name must be {}.toml", p.id),
            ));
        }
        profile.push(p);
    }
    let bundle = Bundle {
        schema: SCHEMA_VERSION,
        revision,
        profile,
    };
    let text = toml::to_string_pretty(&bundle)?;
    Bundle::parse(&text, "generated bundle")?;
    Ok(text)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncOutcome {
    /// Revision before (`None`: no community bundle yet).
    pub from: Option<u32>,
    pub to: u32,
    pub updated: bool,
    pub profiles: usize,
}

/// Install `text` as the community bundle if it is valid and newer (or `force`).
pub fn apply_bundle(dirs: &AppDirs, text: &str, force: bool) -> Result<SyncOutcome> {
    let incoming = Bundle::parse(text, "downloaded bundle")?;
    let dir = community_dir(dirs);
    let path = dir.join(BUNDLE_FILE);
    let current = fs::read_to_string(&path)
        .ok()
        .and_then(|t| Bundle::parse(&t, "current bundle").ok())
        .map(|b| b.revision);
    let updated = force || current.map_or(true, |rev| incoming.revision > rev);
    if updated {
        fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(|e| Error::io(&tmp, e))?;
        fs::rename(&tmp, &path).map_err(|e| Error::io(&path, e))?;
        tracing::info!(from = ?current, to = incoming.revision, "community compat bundle updated");
    }
    Ok(SyncOutcome {
        from: current,
        to: incoming.revision,
        updated,
        profiles: incoming.profile.len(),
    })
}

/// Fetch a community bundle over https and install it with [`apply_bundle`].
pub fn sync_from(dirs: &AppDirs, url: &str, force: bool) -> Result<SyncOutcome> {
    if !url.starts_with("https://") {
        return Err(Error::InvalidInput(format!(
            "bundle URL must use https: {url}"
        )));
    }
    let bytes = cross_engine::download::fetch_bytes(url, MAX_BUNDLE)?
        .ok_or_else(|| Error::InvalidInput(format!("{url} does not exist (404)")))?;
    let text = String::from_utf8(bytes).map_err(|_| Error::invalid(url, "not UTF-8 text"))?;
    apply_bundle(dirs, &text, force)
}

/// Validate a profile file and store it as a local override.
pub fn import_local(dirs: &AppDirs, file: &Path) -> Result<GameProfile> {
    let text = fs::read_to_string(file).map_err(|e| Error::io(file, e))?;
    let profile = GameProfile::parse(&text, &file.display().to_string())?;
    let dir = local_dir(dirs);
    fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let dest = dir.join(format!("{}.toml", profile.id));
    fs::write(&dest, text).map_err(|e| Error::io(&dest, e))?;
    tracing::info!(id = %profile.id, "local compat profile imported");
    Ok(profile)
}

/// Result of [`apply_to_bottle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// Name of the app entry in `bottle.toml` that got the settings.
    pub app: String,
    /// Dependencies of the profile not installed in the bottle yet.
    pub missing_deps: Vec<String>,
}

/// Write `profile`'s settings into `bottle` (not saved): into the Steam game
/// entry for its appid, or into the app entry for `exe` (a Windows or host
/// path). Profile values replace backend/arguments/sync and are added to
/// env and DLL overrides; other settings of the app stay.
pub fn apply_to_bottle(
    bottle: &mut Bottle,
    profile: &GameProfile,
    exe: Option<&Path>,
) -> Result<Applied> {
    let index = match (profile.steam_appid, exe) {
        (_, Some(exe)) => bottle.apps.iter().position(|a| a.path == exe),
        (Some(appid), None) => bottle
            .apps
            .iter()
            .position(|a| a.steam_appid == Some(appid)),
        (None, None) => {
            return Err(Error::InvalidInput(format!(
                "{} is not a Steam game; give the program's path",
                profile.id
            )))
        }
    };
    let index = index.unwrap_or_else(|| {
        bottle.apps.push(App {
            name: profile.name.clone(),
            path: exe.map_or_else(|| cross_steam::STEAM_EXE_WIN.into(), Path::to_owned),
            steam_appid: if exe.is_some() {
                None
            } else {
                profile.steam_appid
            },
            profile: Default::default(),
        });
        bottle.apps.len() - 1
    });
    let from = profile.launch_profile();
    let app = &mut bottle.apps[index];
    if from.backend.is_some() {
        app.profile.backend = from.backend;
    }
    if !from.args.is_empty() {
        app.profile.args = from.args;
    }
    app.profile.env.extend(from.env);
    app.profile.dll_overrides.extend(from.dll_overrides);
    if from.graphics.sync.is_some() {
        app.profile.graphics.sync = from.graphics.sync;
    }
    let missing_deps = profile
        .deps
        .iter()
        .filter(|d| !bottle.deps.contains_key(*d))
        .cloned()
        .collect();
    tracing::info!(bottle = %bottle.id, profile = %profile.id, "compat profile applied");
    Ok(Applied {
        app: bottle.apps[index].name.clone(),
        missing_deps,
    })
}

#[cfg(test)]
mod tests {
    use cross_core::model::{DepRecord, DepSource, GraphicsBackend, SyncMode};

    use super::*;
    use crate::profile::tests::SAMPLE;
    use crate::profile::Rating;

    #[test]
    fn builtin_profiles_are_valid_unverified_and_unique() {
        let db = CompatDb::builtin();
        assert_eq!(db.entries().count(), BUILTIN.len());
        for (id, _) in BUILTIN {
            let e = db
                .get(id)
                .unwrap_or_else(|| panic!("{id}: file id differs"));
            assert!(!e.profile.is_verified(), "{id} must stay unverified");
            assert!(e.profile.source.url.is_some(), "{id} needs a source URL");
        }
        let mut appids: Vec<_> = db.entries().filter_map(|e| e.profile.steam_appid).collect();
        appids.sort_unstable();
        appids.dedup();
        assert_eq!(appids.len(), BUILTIN.len());
        assert_eq!(db.by_appid(367520).unwrap().profile.id, "hollow-knight");
        assert_eq!(db.by_appid(578080).unwrap().profile.rating, Rating::Red);
        assert!(db.by_appid(1).is_none());
    }

    #[test]
    fn local_overrides_community_overrides_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path());

        // Community bundle replaces hollow-knight and adds sample-game.
        let hk = CompatDb::builtin()
            .get("hollow-knight")
            .unwrap()
            .profile
            .clone();
        let community_hk = GameProfile {
            rating: Rating::Yellow,
            ..hk.clone()
        };
        let sample = GameProfile::parse(SAMPLE, "s").unwrap();
        let bundle = Bundle {
            schema: 1,
            revision: 3,
            profile: vec![community_hk, sample.clone()],
        };
        let out = apply_bundle(&dirs, &toml::to_string(&bundle).unwrap(), false).unwrap();
        assert_eq!(
            (out.from, out.to, out.updated, out.profiles),
            (None, 3, true, 2)
        );

        let db = CompatDb::load(&dirs).unwrap();
        assert_eq!(db.get("hollow-knight").unwrap().layer, Layer::Community);
        assert_eq!(
            db.get("hollow-knight").unwrap().profile.rating,
            Rating::Yellow
        );
        assert_eq!(db.get("sample-game").unwrap().layer, Layer::Community);

        // A local file wins; a broken one is reported, not fatal.
        let file = tmp.path().join("mine.toml");
        fs::write(
            &file,
            SAMPLE.replace("rating = \"yellow\"", "rating = \"green\""),
        )
        .unwrap();
        import_local(&dirs, &file).unwrap();
        fs::write(local_dir(&dirs).join("broken.toml"), "schema = 1\n").unwrap();
        let db = CompatDb::load(&dirs).unwrap();
        let s = db.get("sample-game").unwrap();
        assert_eq!(
            (s.layer.clone(), s.profile.rating),
            (Layer::Local, Rating::Green)
        );
        assert_eq!(db.problems.len(), 1, "{:?}", db.problems);

        // Older or equal revisions are ignored unless forced.
        let older = Bundle {
            revision: 2,
            ..bundle.clone()
        };
        let out = apply_bundle(&dirs, &toml::to_string(&older).unwrap(), false).unwrap();
        assert!(!out.updated);
        assert!(
            apply_bundle(&dirs, &toml::to_string(&older).unwrap(), true)
                .unwrap()
                .updated
        );
        assert!(apply_bundle(
            &dirs,
            "schema = 1\nrevision = 9\n[[profile]]\nid = 1",
            false
        )
        .is_err());
    }

    #[test]
    fn sync_requires_https() {
        let tmp = tempfile::tempdir().unwrap();
        let err = sync_from(
            &AppDirs::with_root(tmp.path()),
            "http://x/bundle.toml",
            false,
        )
        .unwrap_err();
        assert!(err.to_string().contains("https"), "{err}");
    }

    #[test]
    fn bundles_are_built_from_profile_files() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("profiles");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("sample-game.toml"), SAMPLE).unwrap();
        let text = make_bundle(&dir, 7).unwrap();
        let bundle = Bundle::parse(&text, "b").unwrap();
        assert_eq!((bundle.revision, bundle.profile.len()), (7, 1));

        fs::write(
            dir.join("wrong-name.toml"),
            SAMPLE.replace("sample-game", "other"),
        )
        .unwrap();
        assert!(make_bundle(&dir, 8).is_err());
        let dup = format!(
            "schema = 1\nrevision = 1\n{}",
            ["[[profile]]", "[[profile]]"]
                .iter()
                .map(|h| format!("{h}\n{}", SAMPLE.replace("[source]", "[profile.source]")))
                .collect::<String>()
        );
        let err = Bundle::parse(&dup, "dup").unwrap_err();
        assert!(err.to_string().contains("duplicate"), "{err}");
    }

    #[test]
    fn exe_matching_prefers_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("Sample.exe");
        fs::write(&exe, b"game").unwrap();
        let hash = cross_engine::download::hash_file(&exe).unwrap();

        let mut db = CompatDb::default();
        let by_name = GameProfile::parse(SAMPLE, "s").unwrap();
        db.insert(by_name.clone(), Layer::Local);
        let m = db.by_exe(&exe).unwrap().unwrap();
        assert_eq!(
            (m.entry.profile.id.as_str(), m.kind),
            ("sample-game", MatchKind::ExeName)
        );

        let by_hash = GameProfile {
            id: "exact".into(),
            exe_names: vec![],
            exe_sha256: vec![hash],
            ..by_name
        };
        db.insert(by_hash, Layer::Local);
        let m = db.by_exe(&exe).unwrap().unwrap();
        assert_eq!(
            (m.entry.profile.id.as_str(), m.kind),
            ("exact", MatchKind::ExeSha256)
        );

        let other = tmp.path().join("Other.exe");
        fs::write(&other, b"x").unwrap();
        assert!(db.by_exe(&other).unwrap().is_none());
    }

    #[test]
    fn applying_writes_the_game_entry_and_lists_missing_deps() {
        let mut bottle = Bottle::new("games", "Games").unwrap();
        bottle.deps.insert(
            "vcrun2022".into(),
            DepRecord {
                installed_unix: 1,
                source: DepSource::Catalog,
            },
        );
        let profile = GameProfile::parse(SAMPLE, "s").unwrap();
        let applied = apply_to_bottle(&mut bottle, &profile, None).unwrap();
        assert_eq!(applied.app, "Sample Game");
        assert_eq!(applied.missing_deps, ["dxvk"]);
        let app = &bottle.apps[0];
        assert_eq!(app.steam_appid, Some(42));
        assert_eq!(app.path, Path::new(cross_steam::STEAM_EXE_WIN));
        assert_eq!(app.profile.backend, Some(GraphicsBackend::Dxvk));
        assert_eq!(app.profile.graphics.sync, Some(SyncMode::Msync));

        // Applying again updates the same entry; an exe gets its own entry.
        apply_to_bottle(&mut bottle, &profile, None).unwrap();
        assert_eq!(bottle.apps.len(), 1);
        apply_to_bottle(
            &mut bottle,
            &profile,
            Some(Path::new(r"C:\Games\Sample.exe")),
        )
        .unwrap();
        assert_eq!(bottle.apps.len(), 2);
        assert_eq!(bottle.apps[1].steam_appid, None);

        let no_steam = GameProfile {
            steam_appid: None,
            ..profile
        };
        assert!(apply_to_bottle(&mut bottle, &no_steam, None).is_err());
    }
}
