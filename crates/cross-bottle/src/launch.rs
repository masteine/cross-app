//! Running programs (`.exe`, `.msi`, `.lnk`, `.bat`) and Wine's built-in tools inside a bottle.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::time::SystemTime;

use cross_core::logging::{timestamp_compact, timestamp_utc};
use cross_core::model::LaunchProfile;

use crate::exec::redact;
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

/// Run logs kept per bottle (`logs/run-*.log`); older ones are deleted.
pub const MAX_RUN_LOGS: usize = 20;
const RUN_LOG_PREFIX: &str = "run-";

/// How a foreground run treats Wine's output.
#[derive(Debug, Clone, Default)]
pub struct RunOptions {
    /// Leave stderr on the terminal instead of writing it to a run log.
    pub no_capture: bool,
    /// `WINEDEBUG` channels for this run, e.g. `+err,+module` (default: `-all`).
    pub wine_debug: Option<String>,
}

#[derive(Debug)]
pub struct RunOutcome {
    pub status: ExitStatus,
    /// The run log holding Wine's stderr, unless capture was off.
    pub log: Option<PathBuf>,
}

/// Run a program in the foreground and return its exit status. Its stdout stays
/// on the terminal; Wine's stderr goes to a run log in the bottle's `logs/`.
pub fn run(runtime: &Runtime, request: &LaunchRequest) -> Result<ExitStatus> {
    Ok(run_with(runtime, request, &RunOptions::default())?.status)
}

/// [`run`] with explicit options.
///
/// stderr is redirected to a file rather than piped through us: Wine hands it
/// to the wineserver and to every program it starts, which can outlive this
/// process, and writing to a pipe nobody reads would kill them with SIGPIPE.
pub fn run_with(
    runtime: &Runtime,
    request: &LaunchRequest,
    options: &RunOptions,
) -> Result<RunOutcome> {
    let mut cmd = build_command(runtime, request)?;
    if let Some(channels) = &options.wine_debug {
        cmd.env("WINEDEBUG", channels);
    }
    if options.no_capture {
        return Ok(RunOutcome {
            status: run_command(cmd)?,
            log: None,
        });
    }

    let logs = logs_dir(runtime);
    fs::create_dir_all(&logs).map_err(|e| Error::io(&logs, e))?;
    let started = SystemTime::now();
    let path = unique_log_path(&logs, &timestamp_compact(started));
    let mut file = OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(&path)
        .map_err(|e| Error::io(&path, e))?;
    let env = cmd
        .get_envs()
        .map(|(k, v)| {
            let value = v.map(|v| v.to_string_lossy()).unwrap_or_default();
            format!("{}={value}", k.to_string_lossy())
        })
        .collect::<Vec<_>>()
        .join(" ");
    let header = format!(
        "# cross-app run log\n# started: {}\n# bottle: {}\n# program: {:?} {}\n# env: {}\n\n",
        timestamp_utc(started),
        runtime.bottle_id(),
        cmd.get_program(),
        redact(&format!("{:?}", cmd.get_args().collect::<Vec<_>>())),
        redact(&env),
    );
    file.write_all(header.as_bytes())
        .map_err(|e| Error::io(&path, e))?;
    let child_stderr = file.try_clone().map_err(|e| Error::io(&path, e))?;
    cmd.stderr(child_stderr);
    tracing::info!(log = %path.display(), "capturing Wine output");

    let status = run_command(cmd)?;
    let elapsed = started.elapsed().map_or(0, |d| d.as_secs());
    let footer = format!("\n# exited: {status} after {elapsed} s\n");
    file.write_all(footer.as_bytes())
        .map_err(|e| Error::io(&path, e))?;
    prune_run_logs(&logs, MAX_RUN_LOGS);
    Ok(RunOutcome {
        status,
        log: Some(path),
    })
}

/// `<bottle>/logs`, next to the prefix.
pub fn logs_dir(runtime: &Runtime) -> PathBuf {
    runtime
        .prefix()
        .parent()
        .unwrap_or_else(|| runtime.prefix())
        .join("logs")
}

fn unique_log_path(dir: &Path, stamp: &str) -> PathBuf {
    let first = dir.join(format!("{RUN_LOG_PREFIX}{stamp}.log"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{RUN_LOG_PREFIX}{stamp}-{n}.log")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

/// Run logs in `dir`, oldest first (names sort by time).
pub fn run_logs(dir: &Path) -> Vec<PathBuf> {
    let mut logs: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(RUN_LOG_PREFIX) && n.ends_with(".log"))
                })
                .collect()
        })
        .unwrap_or_default();
    logs.sort();
    logs
}

fn prune_run_logs(dir: &Path, keep: usize) {
    let logs = run_logs(dir);
    for old in logs.iter().take(logs.len().saturating_sub(keep)) {
        if let Err(e) = fs::remove_file(old) {
            tracing::warn!(path = %old.display(), error = %e, "could not delete old run log");
        }
    }
}

fn run_command(mut cmd: Command) -> Result<ExitStatus> {
    tracing::info!(cmd = %redact(&format!("{cmd:?}")), "launching");
    let started = std::time::Instant::now();
    let status = cmd.status().map_err(|e| Error::Wine {
        what: "launch".into(),
        message: format!("could not start {:?}: {e}", cmd.get_program()),
    })?;
    tracing::info!(%status, elapsed_ms = started.elapsed().as_millis() as u64, "program exited");
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

    /// A wine that prints to both streams and exits with the code in $CODE.
    fn chatty_wine(tmp: &Path) -> Runtime {
        use std::os::unix::fs::PermissionsExt;
        let bin = tmp.join("chatty/bin");
        fs::create_dir_all(&bin).unwrap();
        let wine = bin.join("wine");
        fs::write(
            &wine,
            "#!/bin/sh\necho \"to stdout\"\necho \"err: $WINEDEBUG $*\" >&2\nexit ${CODE:-0}\n",
        )
        .unwrap();
        fs::set_permissions(&wine, fs::Permissions::from_mode(0o755)).unwrap();
        let dirs = AppDirs::with_root(tmp.join("home"));
        fs::create_dir_all(dirs.bottle_prefix("demo")).unwrap();
        Runtime::new(&dirs, "demo", &wine)
    }

    #[test]
    fn run_captures_stderr_into_a_run_log() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = chatty_wine(tmp.path());
        let req = LaunchRequest {
            target: "notepad".into(),
            ..Default::default()
        };
        let out = run_with(
            &rt,
            &req,
            &RunOptions {
                wine_debug: Some("+err".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(out.status.success());

        let log = out.log.unwrap();
        assert_eq!(log.parent().unwrap(), logs_dir(&rt));
        let text = fs::read_to_string(&log).unwrap();
        assert!(text.contains("# bottle: demo"), "{text}");
        assert!(text.contains("err: +err notepad"), "{text}");
        assert!(text.contains("# exited: exit status: 0"), "{text}");
        assert!(
            !text.contains("to stdout"),
            "stdout stays on the terminal: {text}"
        );
    }

    #[test]
    fn run_log_masks_secret_looking_variables() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = chatty_wine(tmp.path());
        let mut profile = LaunchProfile::default();
        profile.env.insert("STEAM_API_KEY".into(), "s3cr3t".into());
        profile.env.insert("DXVK_HUD".into(), "fps".into());
        let req = LaunchRequest {
            target: "notepad".into(),
            profile,
            ..Default::default()
        };
        let log = run_with(&rt, &req, &RunOptions::default())
            .unwrap()
            .log
            .unwrap();
        let text = fs::read_to_string(log).unwrap();
        assert!(!text.contains("s3cr3t"), "{text}");
        assert!(text.contains("STEAM_API_KEY=<redacted>"), "{text}");
        assert!(text.contains("DXVK_HUD=fps"), "{text}");
    }

    #[test]
    fn run_without_capture_writes_no_log() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = chatty_wine(tmp.path());
        let req = LaunchRequest {
            target: "notepad".into(),
            ..Default::default()
        };
        let out = run_with(
            &rt,
            &req,
            &RunOptions {
                no_capture: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(out.log.is_none());
        assert!(run_logs(&logs_dir(&rt)).is_empty());
    }

    #[test]
    fn old_run_logs_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        let rt = chatty_wine(tmp.path());
        let logs = logs_dir(&rt);
        fs::create_dir_all(&logs).unwrap();
        for i in 0..MAX_RUN_LOGS + 5 {
            fs::write(logs.join(format!("run-20200101-0000{i:02}.log")), "").unwrap();
        }
        fs::write(logs.join("cross.log.2026-10-14"), "keep").unwrap();
        let req = LaunchRequest {
            target: "notepad".into(),
            ..Default::default()
        };
        run_with(&rt, &req, &RunOptions::default()).unwrap();

        let remaining = run_logs(&logs);
        assert_eq!(remaining.len(), MAX_RUN_LOGS);
        assert!(
            !logs.join("run-20200101-000000.log").exists(),
            "oldest goes first"
        );
        assert!(
            logs.join("cross.log.2026-10-14").exists(),
            "other logs are untouched"
        );
    }

    #[test]
    fn same_second_runs_get_distinct_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let first = unique_log_path(tmp.path(), "20261014-120000");
        fs::write(&first, "").unwrap();
        let second = unique_log_path(tmp.path(), "20261014-120000");
        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with("-2.log"));
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
