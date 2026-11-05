//! Docker backend for the execution sandbox (Windows and Linux; macOS uses
//! `sandbox-exec`). Each command runs in a fresh container:
//! - `--network none`: no network except the container's own loopback
//! - read-only root file system; writes only to the project copy and the
//!   temp folder (bind mounts) and a small `/tmp`
//! - nothing from the host's home folder is mounted, so secrets can't be read
//! - no capabilities, no privilege escalation, a process and memory limit
//! - a scrubbed environment: only what we pass with `-e`
//! - the timeout stops the whole container
//!
//! Paths: on macOS and Linux the copy, the temp folder and the dependency
//! folders its `node_modules` links point to are mounted at the **same**
//! paths inside the container, so callers' paths and the links work as is.
//! On Windows they're mounted under `/host/<drive>/…`, and paths in the
//! arguments and environment are translated.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::{RunResult, Sandbox};

/// The image commands run in. Full Debian image: it has `sh`, `curl`,
/// `git`, and the compilers native packages need.
pub const DEFAULT_IMAGE: &str = "node:22-bookworm";

pub fn image() -> String {
    capture::settings::var("DOCKER_IMAGE").unwrap_or_else(|| DEFAULT_IMAGE.to_string())
}

/// The Docker daemon answers.
/// Docker is running and runs Linux containers. Docker on Windows in
/// Windows-container mode (GitHub's Windows runners, or a setting in Docker
/// Desktop) can't run the Linux image, so it counts as unavailable.
pub fn available() -> bool {
    Command::new("docker")
        .args(["version", "--format", "{{.Server.Os}}"])
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success() && runs_linux(&String::from_utf8_lossy(&o.stdout)))
}

fn runs_linux(server_os: &str) -> bool {
    server_os.trim().eq_ignore_ascii_case("linux")
}

/// Pull the image once, outside any command's timeout.
fn ensure_image(image: &str) -> Result<()> {
    static DONE: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    DONE.get_or_init(|| {
        let present = Command::new("docker").args(["image", "inspect", image]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        if present {
            return Ok(());
        }
        let out = Command::new("docker").args(["pull", "-q", image]).output().map_err(|e| e.to_string())?;
        if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
    })
    .clone()
    .map_err(|e| anyhow::anyhow!("docker image {image} isn't available: {e}"))
}

/// A Windows path (`C:\a\b` or `C:/a/b`) as a container path (`/host/c/a/b`).
/// None for anything that isn't a drive path.
pub fn windows_container_path(p: &str) -> Option<String> {
    let b = p.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'\\' || b[2] == b'/') {
        let rest = p[2..].replace('\\', "/");
        let rest = rest.trim_end_matches('/');
        return Some(format!("/host/{}{rest}", p[..1].to_ascii_lowercase()));
    }
    None
}

/// Where a host path is inside the container.
fn container_path(host: &Path) -> String {
    let s = host.to_string_lossy();
    if cfg!(windows) { windows_container_path(&s).unwrap_or_else(|| s.replace('\\', "/")) } else { s.into_owned() }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub host: PathBuf,
    pub container: String,
    pub writable: bool,
}

/// Folders outside the copy that its `node_modules` links point into (the
/// project's own `node_modules`, the dependency pool). Their parent
/// `node_modules` folders are mounted read-only, so a linked package can
/// still find its own dependencies next to it.
fn dependency_dirs(work: &Path) -> Vec<PathBuf> {
    let nm = work.join("node_modules");
    let mut links = Vec::new();
    for entry in std::fs::read_dir(&nm).into_iter().flatten().flatten() {
        let path = entry.path();
        if entry.file_name().to_string_lossy().starts_with('@') && path.is_dir() && !path.is_symlink() {
            links.extend(std::fs::read_dir(&path).into_iter().flatten().flatten().map(|e| e.path()));
        } else {
            links.push(path);
        }
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    for link in links {
        let Ok(target) = std::fs::read_link(&link) else { continue };
        let target = if target.is_absolute() { target } else { link.parent().unwrap_or(&nm).join(target) };
        if target.starts_with(work) {
            continue;
        }
        // The `node_modules` folder holding the package (scoped packages
        // sit one level deeper).
        let holder = target.ancestors().find(|a| a.file_name().is_some_and(|n| n == "node_modules")).map(Path::to_path_buf).or_else(|| target.parent().map(Path::to_path_buf));
        if let Some(h) = holder {
            dirs.push(h);
        }
    }
    dirs.sort();
    dirs.dedup();
    // Drop folders inside another mounted folder.
    let all = dirs.clone();
    dirs.retain(|d| !all.iter().any(|o| o != d && d.starts_with(o)));
    dirs
}

pub fn mounts(sb: &Sandbox) -> Vec<Mount> {
    let mut m = vec![
        Mount { host: sb.work.clone(), container: container_path(&sb.work), writable: true },
        Mount { host: sb.tmp.clone(), container: container_path(&sb.tmp), writable: true },
    ];
    // The Linux pool's node_modules, read-only, as the copy's own.
    if let Some(nm) = &sb.node_modules {
        let target = format!("{}/node_modules", container_path(&sb.work));
        m.push(Mount { host: nm.clone(), container: target, writable: false });
    }
    for d in dependency_dirs(&sb.work) {
        if !sb.tmp.starts_with(&d) && !d.starts_with(&sb.tmp) {
            let container = container_path(&d);
            m.push(Mount { host: d, container, writable: false });
        }
    }
    m
}

/// On Windows, replace host paths of the mounts inside `s` with their
/// container paths (`C:\x\work\a.txt` → `/host/c/x/work/a.txt`). Elsewhere
/// the paths are the same and `s` is returned as is.
pub fn translate(s: &str, mounts: &[Mount]) -> String {
    if !cfg!(windows) {
        return s.to_string();
    }
    translate_windows(s, mounts)
}

pub fn translate_windows(s: &str, mounts: &[Mount]) -> String {
    // Every spelling of every mount, longest first, in one pass (a second
    // pass would find `/c/…` again inside `/host/c/…`).
    let mut spellings: Vec<(String, &str)> = Vec::new();
    for m in mounts {
        let host = m.host.to_string_lossy().trim_end_matches(['\\', '/']).to_string();
        for sp in capture::fsx::path_spellings(&host) {
            spellings.push((sp, m.container.as_str()));
        }
    }
    spellings.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then(a.0.cmp(&b.0)));
    if !spellings.iter().any(|(sp, _)| s.contains(sp.as_str())) {
        return s.to_string();
    }
    let alternation: Vec<String> = spellings.iter().map(|(sp, _)| regex::escape(sp)).collect();
    let re = regex::Regex::new(&format!(r#"({})((?:[\\/][^\\/\s"'<>|*?:;,]*)*)"#, alternation.join("|"))).expect("regex");
    re.replace_all(s, |c: &regex::Captures| {
        let container = spellings.iter().find(|(sp, _)| sp == &c[1]).map_or("", |(_, ct)| ct);
        format!("{container}{}", c[2].replace('\\', "/"))
    })
    .into_owned()
}

/// The owner of the copy, so files written in the container belong to the
/// user (Unix hosts).
#[cfg(unix)]
pub(crate) fn user_for(work: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(work).ok().map(|m| format!("{}:{}", m.uid(), m.gid()))
}

#[cfg(not(unix))]
pub(crate) fn user_for(_work: &Path) -> Option<String> {
    None
}

/// The `docker run` arguments for one command.
pub fn run_args(sb: &Sandbox, cwd: &Path, argv: &[&str], env: &[(&str, &str)], name: &str, image: &str) -> Vec<String> {
    let mounts = mounts(sb);
    let tmp = container_path(&sb.tmp);
    let mut a: Vec<String> = [
        "run", "--rm", "--name", name, "--network", "none", "--read-only", "--tmpfs", "/tmp:rw,exec,size=512m", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--pids-limit", "512",
        "--memory", "2g",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(u) = user_for(&sb.work) {
        a.extend(["--user".into(), u]);
    }
    let base_env = [
        ("HOME", tmp.as_str()),
        ("TMPDIR", tmp.as_str()),
        ("NODE_ENV", "test"),
        ("CI", "1"),
        ("npm_config_offline", "true"),
        ("npm_config_ignore_scripts", "true"),
        ("npm_config_update_notifier", "false"),
    ];
    for (k, v) in base_env.iter().copied().chain(env.iter().copied()) {
        a.extend(["-e".into(), format!("{k}={}", translate(v, &mounts))]);
    }
    for m in &mounts {
        let ro = if m.writable { "" } else { ",readonly" };
        a.extend(["--mount".into(), format!("type=bind,source={},target={}{ro}", m.host.to_string_lossy(), m.container)]);
    }
    // Scratch space for tool caches on top of a read-only node_modules.
    if sb.node_modules.is_some() {
        for d in crate::linuxdeps::CACHE_DIRS {
            a.extend(["--mount".into(), format!("type=tmpfs,target={}/node_modules/{d}", container_path(&sb.work))]);
        }
    }
    a.extend(["-w".into(), translate(&cwd.to_string_lossy(), &mounts), image.to_string()]);
    a.extend(argv.iter().map(|s| translate(s, &mounts)));
    a
}

fn container_name() -> String {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    format!("cv-run-{}-{n}-{t}", std::process::id())
}

pub fn run(sb: &Sandbox, cwd: &Path, argv: &[&str], env: &[(&str, &str)], timeout: Duration) -> Result<RunResult> {
    let image = image();
    ensure_image(&image)?;
    let name = container_name();
    let mut cmd = Command::new("docker");
    cmd.args(run_args(sb, cwd, argv, env, &name, &image)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().context("starting docker")?;
    let stop = || {
        let _ = Command::new("docker").args(["kill", &name]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    };
    let r = crate::supervise(child, timeout, &stop)?;
    // 125: `docker run` itself failed (bad mount, daemon error): our
    // environment, not the project.
    if r.exit == Some(125) && !r.timed_out {
        bail!("docker couldn't start the container: {}", r.stderr.trim());
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_linux_containers_count() {
        assert!(runs_linux("linux\n"));
        assert!(!runs_linux("windows\n"));
        assert!(!runs_linux(""));
    }

    #[test]
    fn windows_paths_map_into_the_container() {
        assert_eq!(windows_container_path(r"C:\Users\ann\work").as_deref(), Some("/host/c/Users/ann/work"));
        assert_eq!(windows_container_path("d:/x/").as_deref(), Some("/host/d/x"));
        assert_eq!(windows_container_path("/home/ann"), None);
    }

    #[test]
    fn windows_arguments_are_translated() {
        let mounts = vec![
            Mount { host: PathBuf::from(r"C:\T\cv1\work"), container: "/host/c/T/cv1/work".into(), writable: true },
            Mount { host: PathBuf::from(r"C:\T\cv1\tmp"), container: "/host/c/T/cv1/tmp".into(), writable: true },
        ];
        assert_eq!(translate_windows(r"--outputFile=C:\T\cv1\tmp\vitest-1.json", &mounts), "--outputFile=/host/c/T/cv1/tmp/vitest-1.json");
        assert_eq!(
            translate_windows(r"mkdir -p 'C:\T\cv1\tmp\run-1'; node 'C:\T\cv1\tmp\cv-app-probe.mjs' 1", &mounts),
            "mkdir -p '/host/c/T/cv1/tmp/run-1'; node '/host/c/T/cv1/tmp/cv-app-probe.mjs' 1"
        );
        assert_eq!(translate_windows("npm test", &mounts), "npm test");
        // a sibling folder whose name starts the same isn't a mount
        assert_eq!(translate_windows(r"C:\T\cv1\workshop", &mounts), r"/host/c/T/cv1/workshop");
    }

    #[test]
    fn locked_down_arguments() {
        let sb = Sandbox { work: "/w".into(), tmp: "/t".into(), deny_read: vec![], loopback: true, node_modules: None };
        let a = run_args(&sb, Path::new("/w"), &["npm", "test"], &[("PORT", "3000")], "cv-x", "node:22-bookworm").join(" ");
        for want in ["--network none", "--read-only", "--cap-drop ALL", "no-new-privileges", "--pids-limit 512", "-e HOME=/t", "-e PORT=3000", "type=bind,source=/w,target=/w", "-w /w node:22-bookworm npm test"] {
            assert!(a.contains(want), "{want} missing in {a}");
        }
        assert!(!a.contains("readonly,") && !a.contains("--privileged"));
    }

    #[cfg(unix)]
    #[test]
    fn dependency_links_are_mounted_read_only() {
        let root = tempfile::tempdir().unwrap();
        let pool = root.path().join("pool/G2/node_modules");
        std::fs::create_dir_all(pool.join("vitest")).unwrap();
        std::fs::create_dir_all(pool.join("@scope/pkg")).unwrap();
        let work = root.path().join("work");
        std::fs::create_dir_all(work.join("node_modules/@scope")).unwrap();
        std::os::unix::fs::symlink(pool.join("vitest"), work.join("node_modules/vitest")).unwrap();
        std::os::unix::fs::symlink(pool.join("@scope/pkg"), work.join("node_modules/@scope/pkg")).unwrap();
        let sb = Sandbox { work: work.clone(), tmp: root.path().join("tmp"), deny_read: vec![], loopback: true, node_modules: None };
        let m = mounts(&sb);
        assert_eq!(m.len(), 3, "{m:?}");
        assert_eq!(m[2], Mount { host: pool.clone(), container: pool.to_string_lossy().into_owned(), writable: false });
    }
}
