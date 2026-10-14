//! Installing verbs into a bottle.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cross_bottle::exec::run_captured;
use cross_bottle::registry::RegType;
use cross_bottle::Runtime;
use cross_core::config::BottleStore;
use cross_core::model::{Bottle, DepRecord, DepSource};
use cross_core::paths::AppDirs;
use cross_engine::catalog::{Catalog as EngineCatalog, ComponentKind};
use cross_engine::download::fetch_verified;
use cross_engine::manager::{EngineManager, COMPONENTS_DIR};
use cross_engine::Progress;

use crate::catalog::{parse_component_ref, Download, Step, Verb, VerbCatalog};
use crate::{winetricks, Error, Result};

const INSTALLER_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const SETTLE_TIMEOUT: Duration = Duration::from_secs(120);
const DLL_OVERRIDES_KEY: &str = r"HKCU\Software\Wine\DllOverrides";

#[derive(Debug, Clone, Copy, Default)]
pub struct InstallOptions {
    /// Reinstall even if the bottle already records the verb.
    pub force: bool,
    /// Fall back to the system `winetricks` for verbs outside our catalog.
    pub allow_winetricks: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Installed(DepSource),
    AlreadyInstalled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub verb: String,
    pub status: Status,
}

pub struct Installer<'a> {
    dirs: &'a AppDirs,
    verbs: VerbCatalog,
    engines: EngineCatalog,
    winetricks: Option<PathBuf>,
}

impl<'a> Installer<'a> {
    /// Load the verb and engine catalogs (local overrides included) and look for winetricks.
    pub fn new(dirs: &'a AppDirs) -> Result<Self> {
        Ok(Self::with_parts(
            dirs,
            VerbCatalog::load(dirs)?,
            EngineCatalog::load(dirs)?,
            winetricks::find(),
        ))
    }

    pub fn with_parts(
        dirs: &'a AppDirs,
        verbs: VerbCatalog,
        engines: EngineCatalog,
        winetricks: Option<PathBuf>,
    ) -> Self {
        Self {
            dirs,
            verbs,
            engines,
            winetricks,
        }
    }

    pub fn catalog(&self) -> &VerbCatalog {
        &self.verbs
    }

    pub fn winetricks_path(&self) -> Option<&Path> {
        self.winetricks.as_deref()
    }

    /// Dependencies recorded in a bottle.
    pub fn installed(&self, bottle_id: &str) -> Result<BTreeMap<String, DepRecord>> {
        Ok(Runtime::open(self.dirs, bottle_id)?.0.deps)
    }

    /// Install `verbs` (and what they require) into a bottle, in order. Progress is
    /// saved per verb, so a failure keeps the verbs that already succeeded.
    pub fn install(
        &self,
        bottle_id: &str,
        verbs: &[String],
        options: InstallOptions,
        progress: &mut dyn FnMut(Progress<'_>),
    ) -> Result<Vec<Outcome>> {
        let (_, runtime) = Runtime::open(self.dirs, bottle_id)?;
        let mut outcomes = Vec::new();

        for name in self.verbs.resolve_order(verbs) {
            // Re-read each time: earlier verbs changed the record.
            let bottle = BottleStore::new(self.dirs).load(bottle_id)?;
            if !options.force && bottle.deps.contains_key(name) {
                outcomes.push(Outcome {
                    verb: name.to_owned(),
                    status: Status::AlreadyInstalled,
                });
                continue;
            }

            let source = match self.verbs.get(name) {
                Some(verb) => {
                    for other in &verb.conflicts {
                        if bottle.deps.contains_key(other) {
                            return Err(Error::Conflict {
                                verb: name.to_owned(),
                                other: other.clone(),
                            });
                        }
                    }
                    self.apply(name, verb, &bottle, &runtime, progress)?;
                    DepSource::Catalog
                }
                None => {
                    self.apply_with_winetricks(name, &runtime, options)?;
                    DepSource::Winetricks
                }
            };

            let mut bottle = BottleStore::new(self.dirs).load(bottle_id)?;
            bottle.deps.insert(
                name.to_owned(),
                DepRecord {
                    installed_unix: now_unix(),
                    source,
                },
            );
            BottleStore::new(self.dirs).save(&bottle)?;
            tracing::info!(
                bottle = bottle_id,
                verb = name,
                ?source,
                "dependency installed"
            );
            outcomes.push(Outcome {
                verb: name.to_owned(),
                status: Status::Installed(source),
            });
        }
        Ok(outcomes)
    }

    fn apply_with_winetricks(
        &self,
        name: &str,
        runtime: &Runtime,
        options: InstallOptions,
    ) -> Result<()> {
        crate::catalog::validate_verb_name(name)?;
        let hint = match (&self.winetricks, options.allow_winetricks) {
            (Some(path), true) => return winetricks::run(path, runtime, name),
            (None, true) => {
                " (not in the catalog, and winetricks is not installed: brew install winetricks)"
            }
            (_, false) => " (not in the catalog; winetricks fallback is disabled)",
        };
        Err(Error::UnknownVerb {
            verb: name.to_owned(),
            hint: hint.to_owned(),
        })
    }

    fn apply(
        &self,
        name: &str,
        verb: &Verb,
        bottle: &Bottle,
        runtime: &Runtime,
        progress: &mut dyn FnMut(Progress<'_>),
    ) -> Result<()> {
        tracing::info!(verb = name, title = %verb.title, "installing");
        let work = tempfile::tempdir().map_err(|e| Error::io(std::env::temp_dir(), e))?;

        // Fetch everything first so a bad download fails before the prefix is touched.
        let mut files = BTreeMap::new();
        for download in &verb.downloads {
            files.insert(
                download.id.clone(),
                self.fetch(name, download, work.path(), progress)?,
            );
        }

        for step in &verb.steps {
            match step {
                Step::Run {
                    download,
                    args,
                    ok_exit,
                } => run_installer(name, runtime, &files[download], args, ok_exit, work.path())?,
                Step::Copy { from, to, files } => {
                    let source = self.component_dir(name, bottle, from)?;
                    copy_files(
                        name,
                        &source,
                        &self.dirs.bottle_prefix(&bottle.id).join(to.relative_dir()),
                        files,
                    )?;
                }
                Step::Override { dlls, mode } => {
                    for dll in dlls {
                        runtime.reg_set(DLL_OVERRIDES_KEY, dll, RegType::Sz, mode)?;
                    }
                }
                Step::Registry {
                    key,
                    name: value,
                    kind,
                    data,
                } => {
                    let kind = RegType::parse(kind).ok_or_else(|| {
                        Error::step(name, format!("unknown registry type {kind:?}"))
                    })?;
                    runtime.reg_set(key, value, kind, data)?;
                }
            }
        }
        Ok(())
    }

    /// Download into the shared cache (verified), then place a copy in the work
    /// directory under the installer's own file name.
    fn fetch(
        &self,
        verb: &str,
        download: &Download,
        work: &Path,
        progress: &mut dyn FnMut(Progress<'_>),
    ) -> Result<PathBuf> {
        let archive = cross_engine::catalog::Archive {
            url: download.url.clone(),
            sha256: download.sha256.clone(),
            size: download.size,
        };
        let label = format!("{verb}/{}", download.id);
        let cached = fetch_verified(&archive, &label, &self.dirs.downloads_dir(), progress)?;
        let local = work.join(&download.file);
        fs::copy(&cached, &local).map_err(|e| Error::io(&local, e))?;
        Ok(local)
    }

    /// Folder of the bottle's engine component named by `component:<kind>:<subdir>`.
    fn component_dir(&self, verb: &str, bottle: &Bottle, from: &str) -> Result<PathBuf> {
        let reference = parse_component_ref(from)
            .ok_or_else(|| Error::step(verb, format!("bad copy source {from:?}")))?;
        let engine_id = bottle
            .engine
            .as_deref()
            .ok_or_else(|| Error::step(verb, "the bottle has no engine"))?;
        let engine = EngineManager::new(self.dirs)
            .get(engine_id)?
            .ok_or_else(|| Error::step(verb, format!("engine {engine_id} is not installed")))?;

        let wanted = match reference.kind {
            "dxvk" => ComponentKind::Dxvk,
            "moltenvk" => ComponentKind::Moltenvk,
            other => {
                return Err(Error::step(
                    verb,
                    format!("unknown component kind {other:?}"),
                ))
            }
        };
        let component = engine
            .components
            .iter()
            .find(|cid| self.engines.component(cid).is_some_and(|c| c.kind == wanted))
            .ok_or_else(|| {
                Error::step(
                    verb,
                    format!(
                        "engine {engine_id} has no {} component; run `cross engine install {engine_id} --dxvk`",
                        reference.kind
                    ),
                )
            })?;

        let root = self
            .dirs
            .engine_dir(engine_id)
            .join(COMPONENTS_DIR)
            .join(component);
        locate(&root, reference.subdir).ok_or_else(|| {
            Error::step(
                verb,
                format!(
                    "{} not found inside component {component}",
                    reference.subdir
                ),
            )
        })
    }
}

/// `root/<subdir>`, or `<subdir>` one level down (archives usually have a top folder).
fn locate(root: &Path, subdir: &str) -> Option<PathBuf> {
    let direct = root.join(subdir);
    if direct.is_dir() {
        return Some(direct);
    }
    let mut children: Vec<_> = fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    children.sort();
    children
        .into_iter()
        .map(|c| c.join(subdir))
        .find(|p| p.is_dir())
}

fn run_installer(
    verb: &str,
    runtime: &Runtime,
    file: &Path,
    args: &[String],
    ok_exit: &[i32],
    work: &Path,
) -> Result<()> {
    let name = file
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut cmd = runtime.wine_command();
    cmd.arg(file).args(args).current_dir(work);
    let out = run_captured(cmd, &format!("{verb}: {name}"), INSTALLER_TIMEOUT)?;
    let code = out.status.code();
    if !code.is_some_and(|c| exit_is_ok(c, ok_exit)) {
        return Err(Error::step(
            verb,
            format!(
                "{name} exited with {}: {}",
                code.map_or_else(|| "a signal".to_owned(), |c| c.to_string()),
                out.message()
            ),
        ));
    }

    // Installers spawn helpers; let the wineserver settle so the next step sees the result.
    let mut wait = runtime.wineserver_command();
    wait.arg("-w");
    run_captured(wait, "wineserver -w", SETTLE_TIMEOUT)?;
    Ok(())
}

/// `status` is what Unix reports (low 8 bits); `ok_exit` lists Windows codes.
fn exit_is_ok(status: i32, ok_exit: &[i32]) -> bool {
    ok_exit.iter().any(|ok| ok & 0xff == status)
}

fn copy_files(verb: &str, from: &Path, to: &Path, patterns: &[String]) -> Result<()> {
    fs::create_dir_all(to).map_err(|e| Error::io(to, e))?;
    let mut names: Vec<String> = fs::read_dir(from)
        .map_err(|e| Error::io(from, e))?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort();

    for pattern in patterns {
        let matches: Vec<&String> = names
            .iter()
            .filter(|n| matches_pattern(pattern, n))
            .collect();
        if matches.is_empty() {
            return Err(Error::step(
                verb,
                format!("nothing matches {pattern:?} in {}", from.display()),
            ));
        }
        for name in matches {
            let target = to.join(name);
            fs::copy(from.join(name), &target).map_err(|e| Error::io(target, e))?;
        }
    }
    Ok(())
}

/// Exact name, or `*suffix` (e.g. `*.dll`), case-insensitively.
fn matches_pattern(pattern: &str, name: &str) -> bool {
    match pattern.strip_prefix('*') {
        Some(suffix) => name
            .to_ascii_lowercase()
            .ends_with(&suffix.to_ascii_lowercase()),
        None => name.eq_ignore_ascii_case(pattern),
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use cross_core::model::GraphicsBackend;
    use cross_core::model::Translation;
    use cross_engine::catalog::{Archive, Component, EngineEntry};
    use cross_engine::manager::{InstalledEngine, ENGINE_FILE};

    use super::*;

    const FAKE_WINE: &str = r#"#!/bin/sh
here="$(dirname "$0")"
echo "ARGS:$*" >> "$here/calls.log"
echo "PREFIX:$WINEPREFIX" >> "$here/calls.log"
case "$1" in
  reg)
    if [ "$2" = "add" ]; then echo "$3|$5|$7|$9" >> "$here/reg.db"; fi ;;
  *reboot.exe) exit 3010 ;;
  *newer.exe) exit 1638 ;;
  *fail.exe) echo "installer exploded" >&2; exit 5 ;;
esac
exit 0
"#;

    struct Fixture {
        _tmp: tempfile::TempDir,
        dirs: AppDirs,
        calls: PathBuf,
        reg_db: PathBuf,
    }

    fn script(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A bottle `demo` on a fake engine `eng`; the engine has a DXVK component laid out
    /// like the real archive (`<top>/x86_64-windows`, `<top>/i386-windows`).
    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let dirs = AppDirs::with_root(tmp.path().join("home"));
        let engine_dir = dirs.engine_dir("eng");
        let bin = engine_dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        script(&bin.join("wine"), FAKE_WINE);
        script(
            &bin.join("wineserver"),
            "#!/bin/sh\necho \"SERVER:$*\" >> \"$(dirname \"$0\")/calls.log\"\n",
        );

        for (arch, dlls) in [
            ("x86_64-windows", ["d3d11.dll", "dxgi.dll"]),
            ("i386-windows", ["d3d11.dll", "dxgi.dll"]),
        ] {
            let d = engine_dir.join("components/dxvk-test/dxvk-top").join(arch);
            fs::create_dir_all(&d).unwrap();
            for dll in dlls {
                fs::write(d.join(dll), format!("{arch}/{dll}")).unwrap();
            }
            fs::write(d.join("README.txt"), "not a dll").unwrap();
        }
        let installed = InstalledEngine {
            id: "eng".into(),
            name: "Eng".into(),
            wine_version: "1".into(),
            translation: Translation::Rosetta,
            wine_bin: "bin/wine".into(),
            backends: vec![GraphicsBackend::Dxvk],
            components: vec!["dxvk-test".into()],
            source_sha256: "0".repeat(64),
            installed_unix: 0,
        };
        fs::write(
            engine_dir.join(ENGINE_FILE),
            toml::to_string(&installed).unwrap(),
        )
        .unwrap();

        let mut bottle = Bottle::new("demo", "Demo").unwrap();
        bottle.engine = Some("eng".into());
        BottleStore::new(&dirs).save(&bottle).unwrap();
        fs::create_dir_all(dirs.bottle_prefix("demo")).unwrap();

        Fixture {
            calls: bin.join("calls.log"),
            reg_db: bin.join("reg.db"),
            _tmp: tmp,
            dirs,
        }
    }

    fn engine_catalog() -> EngineCatalog {
        EngineCatalog {
            schema: 1,
            engines: vec![EngineEntry {
                id: "eng".into(),
                name: "Eng".into(),
                wine_version: "1".into(),
                translation: Translation::Rosetta,
                archive: Archive {
                    url: "https://x/e".into(),
                    sha256: "0".repeat(64),
                    size: None,
                },
                backends: vec![],
                components: vec![],
            }],
            components: vec![Component {
                id: "dxvk-test".into(),
                name: "dxvk".into(),
                version: "1".into(),
                kind: ComponentKind::Dxvk,
                archive: Archive {
                    url: "https://x/d".into(),
                    sha256: "0".repeat(64),
                    size: None,
                },
            }],
        }
    }

    fn sha(data: &[u8]) -> String {
        use std::process::{Command, Stdio};
        // No sha2 dependency here: use the system tool.
        let mut child = Command::new("shasum")
            .args(["-a", "256"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), data).unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .to_owned()
    }

    /// Put an "installer" into the download cache so no network is needed.
    fn seed_cache(fx: &Fixture, file: &str, content: &[u8]) -> (String, String) {
        let hash = sha(content);
        let cache = fx.dirs.downloads_dir();
        fs::create_dir_all(&cache).unwrap();
        fs::write(cache.join(format!("{hash}-{file}")), content).unwrap();
        (format!("https://example.invalid/{file}"), hash)
    }

    fn verbs_toml(extra: &str) -> VerbCatalog {
        VerbCatalog::parse(&format!("schema = 1\n{extra}")).unwrap()
    }

    fn installer<'a>(
        fx: &'a Fixture,
        verbs: VerbCatalog,
        winetricks: Option<PathBuf>,
    ) -> Installer<'a> {
        Installer::with_parts(&fx.dirs, verbs, engine_catalog(), winetricks)
    }

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn calls(fx: &Fixture) -> String {
        fs::read_to_string(&fx.calls).unwrap_or_default()
    }

    #[test]
    fn runs_installers_sets_overrides_and_records_the_verb() {
        let fx = fixture();
        let (url, hash) = seed_cache(&fx, "setup.exe", b"installer-bytes");
        let verbs = verbs_toml(&format!(
            r#"
[verbs.rt]
title = "Runtime"
[[verbs.rt.downloads]]
id = "main"
file = "setup.exe"
url = "{url}"
sha256 = "{hash}"
[[verbs.rt.steps]]
kind = "override"
dlls = ["msvcp140", "vcruntime140"]
mode = "native,builtin"
[[verbs.rt.steps]]
kind = "run"
download = "main"
args = ["/q", "/norestart"]
[[verbs.rt.steps]]
kind = "registry"
key = 'HKCU\Software\Cross'
name = "Installed"
type = "dword"
data = "1"
"#
        ));
        let inst = installer(&fx, verbs, None);
        let out = inst
            .install(
                "demo",
                &names(&["rt"]),
                InstallOptions::default(),
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(
            out,
            vec![Outcome {
                verb: "rt".into(),
                status: Status::Installed(DepSource::Catalog)
            }]
        );

        let log = calls(&fx);
        assert!(log.contains("setup.exe /q /norestart"), "{log}");
        assert!(log.contains("SERVER:-w"), "{log}");
        let db = fs::read_to_string(&fx.reg_db).unwrap();
        assert!(
            db.contains(r"HKCU\Software\Wine\DllOverrides|msvcp140|REG_SZ|native,builtin"),
            "{db}"
        );
        assert!(
            db.contains(r"HKCU\Software\Cross|Installed|REG_DWORD|1"),
            "{db}"
        );

        let bottle = BottleStore::new(&fx.dirs).load("demo").unwrap();
        assert_eq!(bottle.deps["rt"].source, DepSource::Catalog);
        assert!(bottle.deps["rt"].installed_unix > 0);
        assert_eq!(inst.installed("demo").unwrap().len(), 1);
    }

    #[test]
    fn second_install_is_skipped_and_force_reruns() {
        let fx = fixture();
        let (url, hash) = seed_cache(&fx, "setup.exe", b"bytes");
        let verbs = verbs_toml(&format!(
            "[verbs.rt]\ntitle=\"R\"\n[[verbs.rt.downloads]]\nid=\"m\"\nfile=\"setup.exe\"\nurl=\"{url}\"\nsha256=\"{hash}\"\n\
             [[verbs.rt.steps]]\nkind=\"run\"\ndownload=\"m\"\n"
        ));
        let inst = installer(&fx, verbs, None);
        let run = |force| {
            inst.install(
                "demo",
                &names(&["rt"]),
                InstallOptions {
                    force,
                    allow_winetricks: false,
                },
                &mut |_| {},
            )
            .unwrap()
        };
        run(false);
        let runs = |fx: &Fixture| calls(fx).matches("setup.exe").count();
        assert_eq!(runs(&fx), 1);
        assert_eq!(run(false)[0].status, Status::AlreadyInstalled);
        assert_eq!(runs(&fx), 1);
        assert!(matches!(run(true)[0].status, Status::Installed(_)));
        assert_eq!(runs(&fx), 2);
    }

    #[test]
    fn exit_codes_are_checked_against_ok_exit() {
        let fx = fixture();
        let mut toml = String::new();
        for (verb, file, ok) in [
            ("reboot", "reboot.exe", "[0, 3010]"),
            ("newer", "newer.exe", "[0]"),
            ("fail", "fail.exe", "[0]"),
        ] {
            let (url, hash) = seed_cache(&fx, file, file.as_bytes());
            toml += &format!(
                "[verbs.{verb}]\ntitle=\"T\"\n[[verbs.{verb}.downloads]]\nid=\"m\"\nfile=\"{file}\"\nurl=\"{url}\"\nsha256=\"{hash}\"\n\
                 [[verbs.{verb}.steps]]\nkind=\"run\"\ndownload=\"m\"\nok_exit={ok}\n"
            );
        }
        let inst = installer(&fx, verbs_toml(&toml), None);
        let go =
            |v: &str| inst.install("demo", &names(&[v]), InstallOptions::default(), &mut |_| {});

        assert!(go("reboot").is_ok());
        // 1638 reaches us as 1638 & 0xff = 102.
        let err = go("newer").unwrap_err().to_string();
        assert!(err.contains("exited with 102"), "{err}");
        let err = go("fail").unwrap_err().to_string();
        assert!(
            err.contains("exited with 5") && err.contains("installer exploded"),
            "{err}"
        );

        let deps = inst.installed("demo").unwrap();
        assert_eq!(deps.keys().collect::<Vec<_>>(), ["reboot"]);
    }

    #[test]
    fn copies_dxvk_dlls_into_the_prefix_and_sets_overrides() {
        let fx = fixture();
        let inst = installer(&fx, VerbCatalog::builtin(), None);
        inst.install(
            "demo",
            &names(&["dxvk"]),
            InstallOptions::default(),
            &mut |_| {},
        )
        .unwrap();

        let win = fx.dirs.bottle_prefix("demo").join("drive_c/windows");
        assert_eq!(
            fs::read_to_string(win.join("system32/d3d11.dll")).unwrap(),
            "x86_64-windows/d3d11.dll"
        );
        assert_eq!(
            fs::read_to_string(win.join("syswow64/dxgi.dll")).unwrap(),
            "i386-windows/dxgi.dll"
        );
        assert!(!win.join("system32/README.txt").exists());

        let db = fs::read_to_string(&fx.reg_db).unwrap();
        for dll in ["d3d10core", "d3d11"] {
            assert!(
                db.contains(&format!(r"DllOverrides|{dll}|REG_SZ|native")),
                "{dll}: {db}"
            );
        }
        assert!(
            !db.contains("DllOverrides|dxgi|"),
            "dxgi must stay builtin: {db}"
        );
    }

    #[test]
    fn dxvk_without_the_component_explains_how_to_get_it() {
        let fx = fixture();
        // Engine installed without DXVK.
        let file = fx.dirs.engine_dir("eng").join(ENGINE_FILE);
        let mut engine: InstalledEngine =
            toml::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        engine.components.clear();
        fs::write(&file, toml::to_string(&engine).unwrap()).unwrap();

        let err = installer(&fx, VerbCatalog::builtin(), None)
            .install(
                "demo",
                &names(&["dxvk"]),
                InstallOptions::default(),
                &mut |_| {},
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("cross engine install eng --dxvk"), "{err}");
        assert!(installer(&fx, VerbCatalog::builtin(), None)
            .installed("demo")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn requirements_install_first_and_conflicts_block() {
        let fx = fixture();
        let step =
            "[[verbs.X.steps]]\nkind=\"registry\"\nkey=\"HKCU\\\\K\"\nname=\"X\"\ndata=\"1\"\n";
        let verbs = verbs_toml(&format!(
            "[verbs.top]\ntitle=\"T\"\nrequires=[\"base\"]\nconflicts=[\"old\"]\n{}\n[verbs.base]\ntitle=\"B\"\n{}\n[verbs.old]\ntitle=\"O\"\n{}",
            step.replace('X', "top"), step.replace('X', "base"), step.replace('X', "old"),
        ));
        let inst = installer(&fx, verbs, None);
        let out = inst
            .install(
                "demo",
                &names(&["top"]),
                InstallOptions::default(),
                &mut |_| {},
            )
            .unwrap();
        assert_eq!(
            out.iter().map(|o| o.verb.as_str()).collect::<Vec<_>>(),
            ["base", "top"]
        );

        // `old` conflicts with nothing yet; once installed, `top --force` is blocked.
        inst.install(
            "demo",
            &names(&["old"]),
            InstallOptions::default(),
            &mut |_| {},
        )
        .unwrap();
        let err = inst
            .install(
                "demo",
                &names(&["top"]),
                InstallOptions {
                    force: true,
                    allow_winetricks: false,
                },
                &mut |_| {},
            )
            .unwrap_err();
        assert!(matches!(err, Error::Conflict { .. }), "{err}");
    }

    #[test]
    fn corrupt_cache_and_no_network_fail_cleanly() {
        let fx = fixture();
        let (url, hash) = seed_cache(&fx, "setup.exe", b"good");
        // Corrupt the cached file: it no longer matches its hash, and the URL is unreachable.
        let cached = fx.dirs.downloads_dir().join(format!("{hash}-setup.exe"));
        fs::write(&cached, b"tampered").unwrap();
        let verbs = verbs_toml(&format!(
            "[verbs.rt]\ntitle=\"R\"\n[[verbs.rt.downloads]]\nid=\"m\"\nfile=\"setup.exe\"\nurl=\"{url}\"\nsha256=\"{hash}\"\n\
             [[verbs.rt.steps]]\nkind=\"run\"\ndownload=\"m\"\n"
        ));
        let inst = installer(&fx, verbs, None);
        assert!(inst
            .install(
                "demo",
                &names(&["rt"]),
                InstallOptions::default(),
                &mut |_| {}
            )
            .is_err());
        assert!(!calls(&fx).contains("setup.exe"), "installer must not run");
        assert!(inst.installed("demo").unwrap().is_empty());
    }

    #[test]
    fn unknown_verbs_use_winetricks_only_when_allowed_and_present() {
        let fx = fixture();
        let tools = fx.dirs.root().join("tools");
        fs::create_dir_all(&tools).unwrap();
        let wt = tools.join("winetricks");
        script(
            &wt,
            &format!(
                "#!/bin/sh\necho \"WT:$*|PREFIX=$WINEPREFIX|WINE=$WINE\" >> \"{}\"\n",
                fx.calls.display()
            ),
        );
        let go = |winetricks: Option<PathBuf>, allow: bool, verb: &str| {
            installer(&fx, VerbCatalog::builtin(), winetricks).install(
                "demo",
                &names(&[verb]),
                InstallOptions {
                    force: false,
                    allow_winetricks: allow,
                },
                &mut |_| {},
            )
        };

        let err = go(None, true, "gdiplus").unwrap_err().to_string();
        assert!(err.contains("winetricks is not installed"), "{err}");
        let err = go(Some(wt.clone()), false, "gdiplus")
            .unwrap_err()
            .to_string();
        assert!(err.contains("disabled"), "{err}");
        assert!(matches!(
            go(Some(wt.clone()), true, "-q").unwrap_err(),
            Error::InvalidVerbName(_)
        ));
        assert!(!calls(&fx).contains("WT:"));

        let out = go(Some(wt), true, "gdiplus").unwrap();
        assert_eq!(out[0].status, Status::Installed(DepSource::Winetricks));
        let log = calls(&fx);
        assert!(log.contains("WT:-q gdiplus|"), "{log}");
        assert!(
            log.contains(&format!(
                "PREFIX={}",
                fx.dirs.bottle_prefix("demo").display()
            )),
            "{log}"
        );
        assert!(log.contains("bin/wine"), "{log}");
        let bottle = BottleStore::new(&fx.dirs).load("demo").unwrap();
        assert_eq!(bottle.deps["gdiplus"].source, DepSource::Winetricks);
    }

    #[test]
    fn missing_bottle_is_reported() {
        let fx = fixture();
        let err = installer(&fx, VerbCatalog::builtin(), None)
            .install(
                "nope",
                &names(&["dxvk"]),
                InstallOptions::default(),
                &mut |_| {},
            )
            .unwrap_err();
        assert!(matches!(err, Error::Bottle(_)), "{err}");
    }

    #[test]
    fn exit_codes_match_modulo_256() {
        assert!(exit_is_ok(0, &[0]));
        assert!(exit_is_ok(194, &[0, 3010]));
        assert!(exit_is_ok(102, &[1638]));
        assert!(!exit_is_ok(1, &[0, 3010]));
        assert!(!exit_is_ok(194, &[0]));
    }

    #[test]
    fn glob_patterns() {
        assert!(matches_pattern("*.dll", "D3D11.DLL"));
        assert!(matches_pattern("d3d11.dll", "D3D11.dll"));
        assert!(!matches_pattern("*.dll", "readme.txt"));
        assert!(!matches_pattern("d3d11.dll", "xd3d11.dll"));
    }
}
