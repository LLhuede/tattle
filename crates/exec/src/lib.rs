//! Opt-in sandboxed execution (Phase 4b, see docs/PHASE_4B_PLAN.md).
//!
//! Runs the project's own commands (tests, build, app start) on a **copy**
//! of the project, inside the macOS sandbox (`sandbox-exec`), with:
//! - no network except loopback (so an app can start and be called locally)
//! - writes only inside the copy and a private temp folder
//! - common secret locations unreadable
//! - a scrubbed environment (`HOME` is the temp folder; no tokens)
//! - a wall-clock timeout that kills the whole process group
//!
//! No AI, never called from hooks or `tattle check`. If no sandbox is
//! available the caller gets [`Backend::Unavailable`] and reports
//! "couldn't verify by running", never a failure. Elsewhere (Windows,
//! Linux) commands run in Docker containers with the same limits
//! ([`docker`]).

use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};

pub mod cross;
pub mod docker;
pub mod linuxdeps;
pub mod verify;
pub mod workspace;

/// How commands are isolated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// macOS `sandbox-exec` with a generated profile.
    MacSandbox,
    /// A locked-down `docker run` per command (see [`docker`]).
    Docker,
    /// Neither is available: execution can't run.
    Unavailable,
}

/// Pick a backend, once per process: the macOS sandbox first, Docker
/// second. `TATTLE_EXEC_BACKEND=docker|macos|none` forces one (e.g. to test the
/// Docker backend on a Mac).
pub fn detect_backend() -> Backend {
    static BACKEND: std::sync::OnceLock<Backend> = std::sync::OnceLock::new();
    BACKEND
        .get_or_init(|| {
            let mac = cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").exists();
            match capture::settings::var("EXEC_BACKEND").as_deref() {
                Some("docker") => return if docker::available() { Backend::Docker } else { Backend::Unavailable },
                Some("macos") => return if mac { Backend::MacSandbox } else { Backend::Unavailable },
                Some("none") => return Backend::Unavailable,
                _ => {}
            }
            if mac {
                Backend::MacSandbox
            } else if docker::available() {
                Backend::Docker
            } else {
                Backend::Unavailable
            }
        })
        .clone()
}

/// Paths under `$HOME` that sandboxed commands may never read.
pub const SECRET_PATHS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".config/gh",
    ".config/gcloud",
    ".kube",
    ".docker",
    ".npmrc",
    ".netrc",
    ".pypirc",
    ".git-credentials",
    ".claude",
    ".claude.json",
    "Library/Keychains",
    "Library/Cookies",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/Firefox",
    "Library/Safari",
    "Library/Application Support/Claude",
];

/// What a sandboxed run may touch.
#[derive(Debug, Clone)]
pub struct Sandbox {
    /// The copy of the project: the only place (besides `tmp`) writes go.
    pub work: PathBuf,
    /// A private temp folder: also `HOME` and `TMPDIR` for the command.
    pub tmp: PathBuf,
    /// Extra paths that must not be readable (on top of [`SECRET_PATHS`]).
    pub deny_read: Vec<PathBuf>,
    /// Allow binding and connecting to localhost (app-start probe).
    pub loopback: bool,
    /// Docker: a Linux `node_modules` (from [`linuxdeps`]) mounted
    /// read-only as the copy's own. Unused by the macOS sandbox.
    pub node_modules: Option<PathBuf>,
}

fn quote(p: &Path) -> String {
    // SBPL string literal.
    format!("\"{}\"", p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""))
}

/// Resolve symlinks so profile paths match what the kernel sees
/// (`/tmp` is `/private/tmp` on macOS).
fn real(p: &Path) -> PathBuf {
    capture::fsx::canonical(p).unwrap_or_else(|_| p.to_path_buf())
}

impl Sandbox {
    /// The `sandbox-exec` profile (SBPL).
    ///
    /// Network rules are deliberately split: a broad `(allow network*
    /// (local ip "localhost:*"))` together with Unix sockets also lets
    /// outbound connections to the internet through (found in the spike).
    pub fn profile(&self, home: &Path) -> String {
        let mut p = String::from("(version 1)\n(allow default)\n(deny network*)\n");
        if self.loopback {
            // Test servers often bind 0.0.0.0 (supertest). Binding is
            // harmless while connections stay localhost-only both ways.
            p.push_str("(allow network-bind (local ip \"*:*\"))\n");
            p.push_str("(allow network-inbound (local ip \"localhost:*\"))\n");
            p.push_str("(allow network-outbound (remote ip \"localhost:*\"))\n");
        }
        // Local IPC (e.g. DNS resolution answers "no network" quickly).
        p.push_str("(allow network-outbound (remote unix-socket))\n");
        // Unix-socket servers inside the copy or the temp folder only (tsx
        // opens an IPC pipe in TMPDIR; found by the app-start probe).
        p.push_str(&format!(
            "(allow network-bind network-inbound (local unix-socket (subpath {})) (local unix-socket (subpath {})))\n",
            quote(&real(&self.tmp)),
            quote(&real(&self.work))
        ));
        p.push_str("(deny file-write*)\n");
        p.push_str(&format!(
            "(allow file-write* (subpath {}) (subpath {}) (literal \"/dev/null\") (literal \"/dev/zero\") (regex #\"^/dev/tty\") (regex #\"^/dev/fd/\"))\n",
            quote(&real(&self.work)),
            quote(&real(&self.tmp))
        ));
        let mut denied: Vec<PathBuf> = SECRET_PATHS.iter().map(|s| home.join(s)).collect();
        denied.extend(self.deny_read.iter().map(|p| real(p)));
        let filters: Vec<String> = denied.iter().map(|p| format!("(subpath {})", quote(p))).collect();
        p.push_str(&format!("(deny file-read* {})\n", filters.join(" ")));
        p
    }
}

/// The result of one sandboxed command.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub exit: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub millis: u128,
}

impl RunResult {
    pub fn ok(&self) -> bool {
        self.exit == Some(0) && !self.timed_out
    }
}

/// Folders holding the tools a command needs (node, npm), for `PATH`.
#[cfg(unix)]
fn tool_path() -> String {
    let mut dirs: Vec<String> = Vec::new();
    for tool in ["node", "npm"] {
        if let Ok(out) = Command::new("/usr/bin/which").arg(tool).output()
            && let Some(dir) = Path::new(String::from_utf8_lossy(&out.stdout).trim()).parent()
        {
            let d = dir.to_string_lossy().into_owned();
            if !d.is_empty() && !dirs.contains(&d) {
                dirs.push(d);
            }
        }
    }
    dirs.extend(["/usr/bin".into(), "/bin".into(), "/usr/sbin".into(), "/sbin".into()]);
    dirs.join(":")
}

/// Run `argv` in `cwd` (inside the sandbox's `work`) in the sandbox, with
/// a scrubbed environment plus `env`, stopping everything it started after
/// `timeout`.
pub fn run(sb: &Sandbox, cwd: &Path, argv: &[&str], env: &[(&str, &str)], timeout: Duration) -> Result<RunResult> {
    match detect_backend() {
        Backend::MacSandbox => run_mac(sb, cwd, argv, env, timeout),
        Backend::Docker => docker::run(sb, cwd, argv, env, timeout),
        Backend::Unavailable => anyhow::bail!("no sandbox to run the project's code in (needs macOS, or Docker)"),
    }
}

#[cfg(not(unix))]
fn run_mac(_sb: &Sandbox, _cwd: &Path, _argv: &[&str], _env: &[(&str, &str)], _timeout: Duration) -> Result<RunResult> {
    anyhow::bail!("the macOS sandbox isn't available on this system")
}

/// Wait for `child` with a timeout, reading its output on threads so a
/// chatty command can't block on a full pipe. `stop` ends everything the
/// command started; it's called on timeout and once more at the end.
pub(crate) fn supervise(mut child: std::process::Child, timeout: Duration, stop: &dyn Fn()) -> Result<RunResult> {
    use std::io::Read;
    use std::time::Instant;
    let started = Instant::now();
    let mut out = child.stdout.take().expect("piped");
    let mut err = child.stderr.take().expect("piped");
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let mut timed_out = false;
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break Some(st);
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            stop();
            break child.wait().ok();
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    // The leader may exit while background children live on: clean up.
    stop();
    Ok(RunResult {
        exit: status.and_then(|s| s.code()),
        stdout: out_t.join().unwrap_or_default(),
        stderr: err_t.join().unwrap_or_default(),
        timed_out,
        millis: started.elapsed().as_millis(),
    })
}

/// The macOS backend: `sandbox-exec` with a generated profile, killing the
/// whole process group.
#[cfg(unix)]
fn run_mac(sb: &Sandbox, cwd: &Path, argv: &[&str], env: &[(&str, &str)], timeout: Duration) -> Result<RunResult> {
    use std::os::unix::process::CommandExt;
    let home = capture::fsx::home_dir().unwrap_or_default();
    let mut cmd = Command::new("/usr/bin/sandbox-exec");
    cmd.arg("-p").arg(sb.profile(&home)).args(argv);
    cmd.current_dir(cwd)
        .env_clear()
        .env("PATH", tool_path())
        .env("HOME", &sb.tmp)
        .env("TMPDIR", &sb.tmp)
        .env("NODE_ENV", "test")
        .env("CI", "1")
        .env("npm_config_offline", "true")
        .env("npm_config_ignore_scripts", "true")
        .env("npm_config_update_notifier", "false")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let child = cmd.spawn().context("starting sandbox-exec")?;
    let pgid = child.id() as i32;
    supervise(child, timeout, &|| kill_group(pgid))
}

#[cfg(unix)]
fn kill_group(pgid: i32) {
    let _ = Command::new("/bin/kill").args(["-KILL", &format!("-{pgid}")]).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Make `dest/node_modules` a real folder whose entries link to the
/// packages in `src` (read-only, outside the copy). Tool caches such as
/// `node_modules/.vite` and `node_modules/.cache` are then written inside
/// the copy instead of being blocked (found in the spike: vitest exits
/// non-zero when it can't write `.vite/vitest/results.json`).
pub fn link_node_modules(src: &Path, dest: &Path) -> Result<()> {
    let nm = dest.join("node_modules");
    std::fs::create_dir_all(&nm)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        // Only packages and `.bin`: dot-folders are tool caches and temp
        // files (`.vite`, `.cache`, `.vite-temp` left by the agent's own
        // runs; real session R07) and must start empty inside the copy.
        if name.to_string_lossy().starts_with('.') && name != ".bin" {
            continue;
        }
        link_dir(&entry.path(), &nm.join(&name)).with_context(|| format!("linking {}", name.to_string_lossy()))?;
    }
    Ok(())
}

/// A symbolic link at `link` pointing to `target`. On Windows this needs
/// Developer Mode (or admin rights); it's only used by the execution layer,
/// which doesn't run there yet.
pub fn link_dir(target: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    return if target.is_dir() { std::os::windows::fs::symlink_dir(target, link) } else { std::os::windows::fs::symlink_file(target, link) };
}

/// Arguments for the Docker fallback: no network, the copy mounted as the
/// only writable folder, a read-only root, and limits. (Spike: built and
/// unit-tested, not yet used.)
pub fn docker_args(image: &str, work: &Path, argv: &[&str]) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "run".into(),
        "--rm".into(),
        "--network".into(),
        "none".into(),
        "--read-only".into(),
        "--tmpfs".into(),
        "/tmp:rw,size=512m".into(),
        "--pids-limit".into(),
        "512".into(),
        "--memory".into(),
        "2g".into(),
        "-e".into(),
        "HOME=/tmp".into(),
        "-e".into(),
        "CI=1".into(),
        "-v".into(),
        format!("{}:/work:rw", work.to_string_lossy()),
        "-w".into(),
        "/work".into(),
        image.into(),
    ];
    a.extend(argv.iter().map(|s| s.to_string()));
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)] // the profile is for macOS; its paths are Unix paths
    fn profile_denies_by_default_and_splits_network_rules() {
        let sb = Sandbox { work: "/w".into(), tmp: "/t".into(), deny_read: vec!["/secret".into()], loopback: true, node_modules: None };
        let p = sb.profile(Path::new("/Users/me"));
        assert!(p.contains("(deny network*)"));
        assert!(p.contains("(deny file-write*)"));
        assert!(p.contains("(allow network-outbound (remote ip \"localhost:*\"))"));
        assert!(!p.contains("(allow network* (local ip"), "broad local rule lets outbound traffic through");
        assert!(p.contains("\"/Users/me/.ssh\"") && p.contains("\"/secret\""));
        let no_loop = Sandbox { loopback: false, ..sb }.profile(Path::new("/Users/me"));
        assert!(!no_loop.contains("network-bind (local ip"), "no IP binding without loopback");
        assert!(no_loop.contains("(local unix-socket (subpath \"/t\"))"), "unix sockets only in the sandbox folders");
    }

    #[test]
    fn docker_fallback_has_no_network() {
        let a = docker_args("node:18", Path::new("/copy"), &["npm", "test"]);
        let joined = a.join(" ");
        assert!(joined.contains("--network none") && joined.contains("--read-only") && joined.ends_with("npm test"));
    }
}
