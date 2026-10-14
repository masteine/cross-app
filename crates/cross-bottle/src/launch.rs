//! Running programs (`.exe`, `.msi`, `.lnk`, `.bat`) and Wine's built-in tools inside a bottle.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

use cross_core::model::LaunchProfile;

use crate::runtime::Runtime;
use crate::{Error, Result};

/// Performance and debugging switches exposed as flags; each maps to one env var.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    /// `WINEESYNC=1`
    pub esync: bool,
    /// `WINEMSYNC=1` (macOS mach-semaphore sync)
    pub msync: bool,
    /// `MTL_HUD_ENABLED=1`: Apple's Metal performance HUD
    pub metal_hud: bool,
    /// `DXVK_HUD=<spec>`, e.g. `fps,frametimes`
    pub dxvk_hud: Option<String>,
}

impl Flags {
    pub fn env(&self) -> Vec<(&'static str, String)> {
        let mut env = Vec::new();
        if self.esync {
            env.push(("WINEESYNC", "1".to_owned()));
        }
        if self.msync {
            env.push(("WINEMSYNC", "1".to_owned()));
        }
        if self.metal_hud {
            env.push(("MTL_HUD_ENABLED", "1".to_owned()));
        }
        if let Some(spec) = &self.dxvk_hud {
            env.push(("DXVK_HUD", spec.clone()));
        }
        env
    }
}

#[derive(Debug, Clone, Default)]
pub struct LaunchRequest {
    /// A host path, a Windows path (`C:\...`), or a bare program name such as `notepad`.
    pub target: String,
    pub args: Vec<String>,
    /// Working directory on the host; defaults to the program's own folder.
    pub working_dir: Option<PathBuf>,
    pub profile: LaunchProfile,
    pub flags: Flags,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Exe,
    Msi,
    Lnk,
    Batch,
}

fn kind_of(path: &str) -> Kind {
    let lower = path.to_ascii_lowercase();
    match lower.rsplit_once('.').map(|(_, ext)| ext) {
        Some("msi") => Kind::Msi,
        Some("lnk") => Kind::Lnk,
        Some("bat" | "cmd") => Kind::Batch,
        _ => Kind::Exe,
    }
}

/// `C:\foo`, `c:/foo` or `\\server\share`.
pub fn looks_like_windows_path(text: &str) -> bool {
    let b = text.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/'))
        || text.starts_with("\\\\")
}

/// Windows path for a host path through Wine's default `Z:` drive (mapped to `/`).
pub fn to_z_drive(path: &Path) -> String {
    format!("Z:{}", path.display().to_string().replace('/', "\\"))
}

/// Build the Wine command line for `request`, without running it.
pub fn build_command(runtime: &Runtime, request: &LaunchRequest) -> Result<Command> {
    let target = request.target.trim();
    if target.is_empty() {
        return Err(Error::InvalidInput("no program given".into()));
    }

    let host = Path::new(target);
    let (wine_args, default_cwd): (Vec<String>, Option<PathBuf>) = if host.exists() {
        let abs = host.canonicalize().map_err(|e| Error::io(host, e))?;
        let cwd = abs
            .is_file()
            .then(|| abs.parent().map(Path::to_owned))
            .flatten();
        let abs_str = abs.display().to_string();
        let args = match kind_of(&abs_str) {
            Kind::Exe => vec![abs_str],
            Kind::Msi => vec!["msiexec".into(), "/i".into(), to_z_drive(&abs)],
            Kind::Lnk => vec!["start".into(), "/unix".into(), abs_str],
            Kind::Batch => vec!["cmd".into(), "/c".into(), to_z_drive(&abs)],
        };
        (args, cwd)
    } else if looks_like_windows_path(target) {
        let args = match kind_of(target) {
            Kind::Exe => vec![target.to_owned()],
            Kind::Msi => vec!["msiexec".into(), "/i".into(), target.to_owned()],
            Kind::Lnk => vec!["start".into(), target.to_owned()],
            Kind::Batch => vec!["cmd".into(), "/c".into(), target.to_owned()],
        };
        (args, None)
    } else if !target.contains(['/', '\\']) {
        // A bare name such as `notepad` or `regedit`: let Wine resolve it.
        (vec![target.to_owned()], None)
    } else {
        return Err(Error::InvalidInput(format!("file not found: {target}")));
    };

    let profile = &request.profile;
    let mut cmd = runtime.wine_command();
    cmd.args(&wine_args).args(&profile.args).args(&request.args);

    let cwd = request
        .working_dir
        .clone()
        .or_else(|| profile.working_dir.clone())
        .or(default_cwd);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }

    cmd.envs(profile.launch_env());
    cmd.envs(request.flags.env());
    Ok(cmd)
}

/// Run a program in the foreground with inherited stdio and return its exit status.
pub fn run(runtime: &Runtime, request: &LaunchRequest) -> Result<ExitStatus> {
    run_command(build_command(runtime, request)?)
}

fn run_command(mut cmd: Command) -> Result<ExitStatus> {
    tracing::info!(?cmd, "launching");
    let status = cmd.status().map_err(|e| Error::Wine {
        what: "launch".into(),
        message: format!("could not start {:?}: {e}", cmd.get_program()),
    })?;
    tracing::info!(%status, "program exited");
    Ok(status)
}

/// Wine's built-in configuration tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Winecfg,
    Regedit,
    Cmd,
    Taskmgr,
}

impl Tool {
    pub fn program(self) -> &'static str {
        match self {
            Tool::Winecfg => "winecfg",
            Tool::Regedit => "regedit",
            Tool::Cmd => "cmd",
            Tool::Taskmgr => "taskmgr",
        }
    }
}

pub fn tool_command(runtime: &Runtime, tool: Tool) -> Command {
    let mut cmd = runtime.wine_command();
    cmd.arg(tool.program());
    cmd
}

/// Run a tool in the foreground (`cmd` is interactive when stdio is a terminal).
pub fn run_tool(runtime: &Runtime, tool: Tool) -> Result<ExitStatus> {
    run_command(tool_command(runtime, tool))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::{OsStr, OsString};
    use std::fs;

    use cross_core::paths::AppDirs;

    use super::*;
    use crate::runtime::test_support::fake_wine;

    fn runtime(tmp: &Path) -> Runtime {
        let fake = fake_wine(tmp);
        Runtime::new(&AppDirs::with_root(tmp), "demo", &fake.wine)
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn envs(cmd: &Command) -> HashMap<OsString, Option<OsString>> {
        cmd.get_envs()
            .map(|(k, v)| (k.to_owned(), v.map(OsStr::to_owned)))
            .collect()
    }

    fn file(tmp: &Path, name: &str) -> PathBuf {
        let p = tmp.join(name);
        fs::write(&p, "").unwrap();
        p.canonicalize().unwrap()
    }

    #[test]
    fn exe_runs_directly_in_its_own_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime(tmp.path());
        let exe = file(tmp.path(), "app.exe");
        let req = LaunchRequest {
            target: exe.display().to_string(),
            args: vec!["--fast".into(), "x y".into()],
            ..Default::default()
        };
        let cmd = build_command(&rt, &req).unwrap();
        assert_eq!(
            args(&cmd),
            [exe.display().to_string().as_str(), "--fast", "x y"]
        );
        assert_eq!(cmd.get_current_dir(), exe.parent());
    }

    #[test]
    fn msi_lnk_and_bat_get_the_right_wrapper() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime(tmp.path());
        let build = |name: &str| {
            let p = file(tmp.path(), name);
            let cmd = build_command(
                &rt,
                &LaunchRequest {
                    target: p.display().to_string(),
                    ..Default::default()
                },
            )
            .unwrap();
            (p, args(&cmd))
        };

        let (p, a) = build("setup.MSI");
        assert_eq!(a, ["msiexec", "/i", &to_z_drive(&p)]);
        let (p, a) = build("game.lnk");
        assert_eq!(a, ["start", "/unix", &p.display().to_string()]);
        let (p, a) = build("run.bat");
        assert_eq!(a, ["cmd", "/c", &to_z_drive(&p)]);
    }

    #[test]
    fn windows_paths_and_bare_names_pass_through() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime(tmp.path());
        let build = |target: &str| {
            let cmd = build_command(
                &rt,
                &LaunchRequest {
                    target: target.into(),
                    ..Default::default()
                },
            )
            .unwrap();
            (args(&cmd), cmd.get_current_dir().map(Path::to_owned))
        };
        assert_eq!(
            build(r"C:\windows\system32\cmd.exe"),
            (vec![r"C:\windows\system32\cmd.exe".to_string()], None)
        );
        assert_eq!(build("notepad"), (vec!["notepad".to_string()], None));
        assert_eq!(build(r"C:\setup.msi").0, ["msiexec", "/i", r"C:\setup.msi"]);
    }

    #[test]
    fn missing_files_and_empty_targets_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime(tmp.path());
        for target in ["", "  ", "/no/such/app.exe", "dir/app.exe"] {
            let err = build_command(
                &rt,
                &LaunchRequest {
                    target: target.into(),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(matches!(err, Error::InvalidInput(_)), "{target:?}: {err}");
        }
    }

    #[test]
    fn profile_env_flags_and_workdir_are_applied() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime(tmp.path());
        let exe = file(tmp.path(), "game.exe");
        let workdir = tmp.path().join("work");
        fs::create_dir(&workdir).unwrap();

        let mut profile = LaunchProfile {
            args: vec!["-profile-arg".into()],
            ..Default::default()
        };
        profile.env.insert("CUSTOM".into(), "1".into());
        profile.dll_overrides.insert("d3d11".into(), "n,b".into());

        let req = LaunchRequest {
            target: exe.display().to_string(),
            args: vec!["-cli-arg".into()],
            working_dir: Some(workdir.clone()),
            profile,
            flags: Flags {
                msync: true,
                metal_hud: true,
                dxvk_hud: Some("fps".into()),
                ..Default::default()
            },
        };
        let cmd = build_command(&rt, &req).unwrap();
        assert_eq!(args(&cmd)[1..], ["-profile-arg", "-cli-arg"]);
        assert_eq!(cmd.get_current_dir(), Some(workdir.as_path()));

        let env = envs(&cmd);
        let get = |k: &str| {
            env.get(OsStr::new(k))
                .cloned()
                .flatten()
                .map(|v| v.to_string_lossy().into_owned())
        };
        assert_eq!(get("WINEMSYNC").as_deref(), Some("1"));
        assert_eq!(get("MTL_HUD_ENABLED").as_deref(), Some("1"));
        assert_eq!(get("DXVK_HUD").as_deref(), Some("fps"));
        assert_eq!(get("WINEDLLOVERRIDES").as_deref(), Some("d3d11=n,b"));
        assert_eq!(get("CUSTOM").as_deref(), Some("1"));
        assert_eq!(get("WINEESYNC"), None);
    }

    #[test]
    fn runs_through_the_fake_wine_and_reports_the_exit_status() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = fake_wine(tmp.path());
        let rt = Runtime::new(&AppDirs::with_root(tmp.path()), "demo", &fake.wine);
        let exe = file(tmp.path(), "app.exe");
        let status = run(
            &rt,
            &LaunchRequest {
                target: exe.display().to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(status.success());
        assert!(fake.calls().contains(&format!("ARGS:{}", exe.display())));
    }

    #[test]
    fn tools_map_to_wine_programs() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = runtime(tmp.path());
        for (tool, name) in [
            (Tool::Winecfg, "winecfg"),
            (Tool::Regedit, "regedit"),
            (Tool::Cmd, "cmd"),
            (Tool::Taskmgr, "taskmgr"),
        ] {
            assert_eq!(args(&tool_command(&rt, tool)), [name]);
        }
    }

    #[test]
    fn windows_path_detection() {
        assert!(looks_like_windows_path(r"C:\x"));
        assert!(looks_like_windows_path("d:/x"));
        assert!(looks_like_windows_path(r"\\srv\share"));
        assert!(!looks_like_windows_path("/usr/bin/x"));
        assert!(!looks_like_windows_path("notepad"));
        assert!(!looks_like_windows_path("C:"));
    }
}
