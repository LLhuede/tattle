//! Linux dependencies for the Docker backend: one `node_modules` per
//! lockfile, kept in a pool (`~/.cv/linux-deps/<key>/`) and mounted
//! read-only into each container as the copy's `node_modules`.
//!
//! The recordings' own dependency folders hold macOS builds, and a Windows
//! `node_modules` holds Windows builds; the containers run Linux. So the
//! pool is built once per lockfile, only with the user's opt-in
//! (`tattle verify --allow-install`, decided 2026-10-29):
//! 1. **download:** `npm ci --ignore-scripts` in a container with network.
//!    Install scripts are off, so neither the project's code nor any
//!    package's code runs while the network is on. Only npm itself runs.
//! 2. **build:** packages with native code (`binding.gyp`) are compiled
//!    with `npm rebuild` in a container with the network **off**, against
//!    the image's own Node headers.
//! 3. the result is moved into place in one rename, with `ready.json`.
//!
//! Anything that fails leaves no pool entry, and the session is reported as
//! "couldn't verify (dependencies)", never as a failure.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::json;

/// Folders tools write caches into inside `node_modules`. They get a
/// writable scratch mount on top of the read-only pool (vitest exits
/// non-zero when it can't write `.vite/vitest/results.json`).
pub const CACHE_DIRS: &[&str] = &[".vite", ".vite-temp", ".cache", ".tmp"];

/// Where the pool lives: `TATTLE_LINUX_DEPS`, else `~/.cv/linux-deps`.
pub fn default_root() -> PathBuf {
    if let Some(p) = capture::settings::var_os("LINUX_DEPS") {
        return PathBuf::from(p);
    }
    capture::fsx::home_dir().unwrap_or_else(std::env::temp_dir).join(".cv").join("linux-deps")
}

#[derive(Debug, Clone)]
pub struct Options {
    pub root: PathBuf,
    /// Allowed to download (step 1). Without it, only ready pool entries
    /// are used.
    pub allow_install: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { root: default_root(), allow_install: false }
    }
}

/// The pool key: the lockfile and the image (native builds depend on the
/// image's Node version).
pub fn key(lockfile: &[u8], image: &str) -> String {
    let mut bytes = lockfile.to_vec();
    bytes.extend_from_slice(b"\0");
    bytes.extend_from_slice(image.as_bytes());
    capture::text::sha256_hex(&bytes)[..16].to_string()
}

/// Package folders under `nm` that compile native code, relative to `nm`
/// (e.g. `better-sqlite3`, `a/node_modules/b`). Sorted.
pub fn native_packages(nm: &Path) -> Vec<String> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if name.starts_with('.') || !path.is_dir() || path.is_symlink() {
                continue;
            }
            if name.starts_with('@') {
                walk(&path, &format!("{rel}{name}/"), out);
                continue;
            }
            let pkg = format!("{rel}{name}");
            if path.join("binding.gyp").is_file() {
                out.push(pkg.clone());
            }
            let nested = path.join("node_modules");
            if nested.is_dir() {
                walk(&nested, &format!("{pkg}/node_modules/"), out);
            }
        }
    }
    let mut out = Vec::new();
    walk(nm, "", &mut out);
    out
}

/// The package name of a native package path (`a/node_modules/@s/b` →
/// `@s/b`), for `npm rebuild`.
fn package_name(rel: &str) -> &str {
    rel.rsplit("node_modules/").next().unwrap_or(rel)
}

/// What the download step needs to reach the registry.
struct Proxy {
    /// `-e` pairs.
    env: Vec<(String, String)>,
    /// (host file, container path), read-only.
    mounts: Vec<(PathBuf, String)>,
    /// The proxy is on this machine's loopback: use the host's network.
    host_net: bool,
}

/// Proxy settings and a CA bundle for the download step, from the host's
/// environment (corporate proxies; the cloud session's proxy).
fn proxy_settings() -> Proxy {
    let mut env = Vec::new();
    let mut host_net = false;
    for k in ["HTTPS_PROXY", "HTTP_PROXY", "NO_PROXY", "https_proxy", "http_proxy", "no_proxy"] {
        if let Ok(v) = std::env::var(k)
            && !v.is_empty()
        {
            // A proxy on this machine's loopback is only reachable with the
            // host's network.
            if k.to_lowercase() != "no_proxy" && (v.contains("://127.") || v.contains("://localhost")) {
                host_net = true;
            }
            env.push((k.to_string(), v.clone()));
            if k.eq_ignore_ascii_case("https_proxy") {
                env.push(("npm_config_https_proxy".into(), v));
            } else if k.eq_ignore_ascii_case("http_proxy") {
                env.push(("npm_config_proxy".into(), v));
            }
        }
    }
    let mut mounts = Vec::new();
    for k in ["NODE_EXTRA_CA_CERTS", "SSL_CERT_FILE"] {
        if let Some(p) = std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_file()) {
            mounts.push((p, "/cv-ca.pem".to_string()));
            env.push(("NODE_EXTRA_CA_CERTS".into(), "/cv-ca.pem".into()));
            break;
        }
    }
    env.sort();
    env.dedup();
    Proxy { env, mounts, host_net }
}

/// `docker run` arguments for one pool step. `online` gives the container a
/// network (download step); otherwise `--network none`.
pub fn step_args(stage: &Path, cache: &Path, image: &str, name: &str, online: bool, argv: &[&str], env: &[(&str, &str)]) -> Vec<String> {
    let mut a: Vec<String> = [
        "run", "--rm", "--name", name, "--read-only", "--tmpfs", "/tmp:rw,exec,size=1g", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--pids-limit", "512", "--memory", "4g",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(u) = crate::docker::user_for(stage) {
        a.extend(["--user".into(), u]);
    }
    let mut envs: Vec<(String, String)> = vec![
        ("HOME".into(), "/tmp".into()),
        ("npm_config_cache".into(), "/cv-npm-cache".into()),
        ("npm_config_update_notifier".into(), "false".into()),
        ("npm_config_fund".into(), "false".into()),
        ("npm_config_audit".into(), "false".into()),
    ];
    let mut mounts: Vec<(PathBuf, String, bool)> = vec![(stage.to_path_buf(), "/deps".into(), true), (cache.to_path_buf(), "/cv-npm-cache".into(), true)];
    if online {
        let proxy = proxy_settings();
        envs.extend(proxy.env);
        mounts.extend(proxy.mounts.into_iter().map(|(h, c)| (h, c, false)));
        if proxy.host_net {
            a.extend(["--network".into(), "host".into()]);
        }
    } else {
        a.extend(["--network".into(), "none".into()]);
    }
    envs.extend(env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    for (k, v) in envs {
        a.extend(["-e".into(), format!("{k}={v}")]);
    }
    for (host, target, rw) in mounts {
        a.extend(["--mount".into(), format!("type=bind,source={},target={target}{}", host.to_string_lossy(), if rw { "" } else { ",readonly" })]);
    }
    a.extend(["-w".into(), "/deps".into(), image.to_string()]);
    a.extend(argv.iter().map(|s| s.to_string()));
    a
}

fn run_step(args: Vec<String>, name: &str, timeout: Duration) -> Result<crate::RunResult> {
    let child = Command::new("docker").args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().context("starting docker")?;
    let stop = || {
        let _ = Command::new("docker").args(["kill", name]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    };
    crate::supervise(child, timeout, &stop)
}

fn last_error(r: &crate::RunResult) -> String {
    let text = format!("{}\n{}", r.stdout, r.stderr);
    let line = text.lines().rev().find(|l| l.contains("ERR") || l.contains("rror")).unwrap_or("").trim();
    if r.timed_out { "timed out".to_string() } else { line.chars().take(200).collect() }
}

/// The pool's `node_modules` for the copy in `work`: ready, or built now if
/// allowed. `Ok(Err(reason))` when it can't be had (never a failure).
pub fn ensure(work: &Path, opts: &Options) -> Result<std::result::Result<PathBuf, String>> {
    let Ok(lock) = std::fs::read(work.join("package-lock.json")) else {
        return Ok(Err("no package-lock.json (the Linux dependency pool needs one)".into()));
    };
    let image = crate::docker::image();
    let dir = opts.root.join(key(&lock, &image));
    if dir.join("ready.json").is_file() {
        return Ok(Ok(dir.join("node_modules")));
    }
    if !opts.allow_install {
        return Ok(Err("Linux dependencies for this lockfile aren't in the pool yet (run tattle verify with --allow-install)".into()));
    }
    std::fs::create_dir_all(&opts.root)?;
    let cache = opts.root.join("npm-cache");
    std::fs::create_dir_all(&cache)?;
    let stage = opts.root.join(format!("{}.partial-{}", dir.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    let _ = capture::fsx::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let result = build(work, &stage, &cache, &image);
    match result {
        Ok(Ok(natives)) => {
            std::fs::write(stage.join("ready.json"), serde_json::to_string_pretty(&json!({"image": image, "native": natives}))? + "\n")?;
            if std::fs::rename(&stage, &dir).is_err() {
                // Another run finished first; use its copy.
                let _ = capture::fsx::remove_dir_all(&stage);
            }
            Ok(Ok(dir.join("node_modules")))
        }
        Ok(Err(why)) => {
            let _ = capture::fsx::remove_dir_all(&stage);
            Ok(Err(why))
        }
        Err(e) => {
            let _ = capture::fsx::remove_dir_all(&stage);
            Err(e)
        }
    }
}

fn build(work: &Path, stage: &Path, cache: &Path, image: &str) -> Result<std::result::Result<Vec<String>, String>> {
    for f in ["package.json", "package-lock.json"] {
        std::fs::copy(work.join(f), stage.join(f)).with_context(|| format!("copying {f}"))?;
    }
    let name = format!("cv-deps-{}-{}", std::process::id(), stage.file_name().unwrap_or_default().to_string_lossy().chars().take(16).collect::<String>());
    // 1. Download, scripts off.
    let args = step_args(stage, cache, image, &name, true, &["npm", "ci", "--ignore-scripts", "--no-audit", "--no-fund"], &[("npm_config_ignore_scripts", "true")]);
    let r = run_step(args, &name, Duration::from_secs(900))?;
    if !r.ok() {
        return Ok(Err(format!("npm ci (download) failed: {}", last_error(&r))));
    }
    let nm = stage.join("node_modules");
    // 2. Native packages, network off.
    let natives = native_packages(&nm);
    if !natives.is_empty() {
        let mut names: Vec<&str> = natives.iter().map(|n| package_name(n)).collect();
        names.sort();
        names.dedup();
        let mut argv = vec!["npm", "rebuild"];
        argv.extend(names.iter().copied());
        let args = step_args(stage, cache, image, &name, false, &argv, &[("npm_config_ignore_scripts", "false"), ("npm_config_nodedir", "/usr/local"), ("npm_config_offline", "true")]);
        let r = run_step(args, &name, Duration::from_secs(900))?;
        let unbuilt: Vec<&String> = natives.iter().filter(|n| !(nm.join(n).join("build/Release").is_dir() || nm.join(n).join("prebuilds").is_dir())).collect();
        if !r.ok() || !unbuilt.is_empty() {
            let which = unbuilt.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
            return Ok(Err(format!("native packages didn't build offline ({which}): {}", last_error(&r))));
        }
    }
    for d in CACHE_DIRS {
        std::fs::create_dir_all(nm.join(d))?;
    }
    Ok(Ok(natives))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_depend_on_lockfile_and_image() {
        let a = key(b"{\"lockfileVersion\":3}", "node:22-bookworm");
        assert_eq!(a.len(), 16);
        assert_eq!(a, key(b"{\"lockfileVersion\":3}", "node:22-bookworm"));
        assert_ne!(a, key(b"{\"lockfileVersion\":2}", "node:22-bookworm"));
        assert_ne!(a, key(b"{\"lockfileVersion\":3}", "node:20-bookworm"));
    }

    #[test]
    fn finds_native_packages_at_any_depth() {
        let root = tempfile::tempdir().unwrap();
        let nm = root.path();
        for d in ["better-sqlite3", "a/node_modules/bcrypt", "@scope/native", "plain", ".bin/x"] {
            std::fs::create_dir_all(nm.join(d)).unwrap();
        }
        for d in ["better-sqlite3", "a/node_modules/bcrypt", "@scope/native", ".bin/x"] {
            std::fs::write(nm.join(d).join("binding.gyp"), "{}").unwrap();
        }
        assert_eq!(native_packages(nm), vec!["@scope/native", "a/node_modules/bcrypt", "better-sqlite3"]);
        assert_eq!(package_name("a/node_modules/bcrypt"), "bcrypt");
        assert_eq!(package_name("x/node_modules/@s/b"), "@s/b");
    }

    #[test]
    fn download_step_is_the_only_one_with_network() {
        let stage = Path::new("/pool/k.partial");
        let cache = Path::new("/pool/npm-cache");
        let on = step_args(stage, cache, "node:22-bookworm", "n", true, &["npm", "ci"], &[]).join(" ");
        let off = step_args(stage, cache, "node:22-bookworm", "n", false, &["npm", "rebuild", "x"], &[("npm_config_ignore_scripts", "false")]).join(" ");
        assert!(!on.contains("--network none"));
        assert!(off.contains("--network none") && off.contains("npm_config_ignore_scripts=false"));
        for a in [&on, &off] {
            assert!(a.contains("--read-only") && a.contains("--cap-drop ALL") && a.contains("target=/deps") && a.contains("target=/cv-npm-cache"));
        }
    }
}
