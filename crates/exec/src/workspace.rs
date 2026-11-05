//! Phase 4b.1: turn a recorded snapshot into a runnable copy, without
//! network.
//!
//! 1. [`materialise`]: the code at any snapshot commit, via `git archive`
//!    (binary files included).
//! 2. [`resolve_dependencies`]: in order:
//!    - **link** an existing `node_modules` whose installed versions match
//!      the lockfile exactly (the user's project, or a pool of other
//!      projects)
//!    - **offline install** from the user's npm cache (`npm ci --offline
//!      --ignore-scripts`, read-only cache, inside the sandbox), then swap in
//!      already-built copies of native packages of the same version
//!    - otherwise "couldn't verify", never a failure
//!
//!    Under Docker the copy always uses the Linux pool instead
//!    ([`crate::linuxdeps`]): the other sources hold macOS or Windows builds.
//! 3. [`test_command`] and [`run_tests`]: the project's own test command in
//!    the sandbox, with a parsed pass/fail summary.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Value, json};

use crate::{RunResult, Sandbox, link_node_modules, run};

/// Write the project as it was at `commit` into `dest` (must be empty).
pub fn materialise(git_dir: &Path, commit: &str, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    let mut archive = Command::new("git")
        .args(["--git-dir"])
        .arg(git_dir)
        .args(["archive", "--format=tar", commit])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("git archive")?;
    let tar = Command::new("tar")
        .args(["-xf", "-", "-C"])
        .arg(dest)
        .stdin(archive.stdout.take().expect("piped"))
        .status()
        .context("tar")?;
    let git = archive.wait()?;
    if !git.success() || !tar.success() {
        bail!("couldn't extract snapshot {commit}");
    }
    Ok(())
}

/// How the copy got its dependencies.
#[derive(Debug, Clone, PartialEq)]
pub enum Deps {
    /// No dependencies declared.
    NoneNeeded,
    /// Linked from an existing, version-matching `node_modules`.
    Linked(PathBuf),
    /// Installed offline from the npm cache (native packages swapped in
    /// from `native_from`).
    Offline { native_from: Vec<PathBuf> },
    /// Docker: the Linux pool's `node_modules`, mounted read-only.
    Pool(PathBuf),
    /// Couldn't get them without network: the run can't be verified.
    Unavailable(String),
}

impl Deps {
    pub fn runnable(&self) -> bool {
        !matches!(self, Deps::Unavailable(_))
    }

    pub fn to_json(&self) -> Value {
        match self {
            Deps::NoneNeeded => json!({"strategy": "none-needed"}),
            Deps::Linked(p) => json!({"strategy": "linked", "from": p}),
            Deps::Offline { native_from } => json!({"strategy": "offline-install", "native_from": native_from}),
            Deps::Pool(p) => json!({"strategy": "linux-pool", "from": p}),
            Deps::Unavailable(why) => json!({"strategy": "unavailable", "reason": why}),
        }
    }
}

fn read_json(p: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(p).ok()?).ok()
}

/// Declared dependency names (all dependency fields).
fn declared(pkg: &Value) -> Vec<String> {
    ["dependencies", "devDependencies", "optionalDependencies"]
        .iter()
        .filter_map(|f| pkg.get(*f).and_then(Value::as_object))
        .flat_map(|o| o.keys().cloned())
        .collect()
}

/// Top-level packages the lockfile pins: name → version.
fn locked(lock: &Value) -> BTreeMap<String, String> {
    lock.get("packages")
        .and_then(Value::as_object)
        .map(|pk| {
            pk.iter()
                .filter_map(|(k, v)| {
                    let name = k.strip_prefix("node_modules/")?;
                    if name.contains("/node_modules/") || v.get("optional") == Some(&Value::Bool(true)) {
                        return None;
                    }
                    Some((name.to_string(), v.get("version")?.as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn installed_version(nm: &Path, name: &str) -> Option<String> {
    read_json(&nm.join(name).join("package.json")).and_then(|p| p.get("version")?.as_str().map(str::to_string))
}


/// Does `nm` hold exactly what the project needs? With a lockfile: every
/// top-level locked package at its locked version. Without: every declared
/// dependency present.
pub fn matches(nm: &Path, pkg: &Value, lock: Option<&Value>) -> bool {
    if !nm.is_dir() {
        return false;
    }
    match lock.map(locked) {
        Some(l) if !l.is_empty() => l.iter().all(|(n, v)| installed_version(nm, n).as_deref() == Some(v.as_str())),
        _ => declared(pkg).iter().all(|n| nm.join(n).join("package.json").is_file()),
    }
}

/// `node_modules` folders to try linking from: each pool folder's
/// `*/node_modules` and `*/*/node_modules` (e.g. `~/Code/cv-sandbox`).
pub fn candidates(pool: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in pool {
        let Ok(entries) = fs::read_dir(root) else { continue };
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for d in dirs {
            if d.join("node_modules").is_dir() {
                out.push(d.join("node_modules"));
            } else if let Ok(sub) = fs::read_dir(&d) {
                let mut subs: Vec<PathBuf> = sub.flatten().map(|e| e.path().join("node_modules")).filter(|p| p.is_dir()).collect();
                subs.sort();
                out.extend(subs);
            }
        }
    }
    out
}

/// A package that compiles native code: a `binding.gyp`.
fn is_native(pkg_dir: &Path) -> bool {
    pkg_dir.join("binding.gyp").is_file()
}

/// Built output present (node-gyp's `build/Release` or `prebuilds/`).
fn is_built(pkg_dir: &Path) -> bool {
    pkg_dir.join("build/Release").is_dir() || pkg_dir.join("prebuilds").is_dir()
}

/// Get the copy's dependencies without network. `preferred` are tried
/// first (the project's own folder), then `pool`. Under Docker: the Linux
/// pool (`linux`), and the caller mounts [`Deps::Pool`] with
/// [`Sandbox::node_modules`].
pub fn resolve_dependencies(work: &Path, sb: &Sandbox, preferred: &[PathBuf], pool: &[PathBuf], linux: &crate::linuxdeps::Options) -> Result<Deps> {
    let Some(pkg) = read_json(&work.join("package.json")) else {
        return Ok(Deps::NoneNeeded);
    };
    if declared(&pkg).is_empty() {
        return Ok(Deps::NoneNeeded);
    }
    if crate::detect_backend() == crate::Backend::Docker {
        return Ok(match crate::linuxdeps::ensure(work, linux)? {
            Ok(nm) => {
                // The mount point for the pool.
                fs::create_dir_all(work.join("node_modules"))?;
                Deps::Pool(nm)
            }
            Err(why) => Deps::Unavailable(why),
        });
    }
    let lock = read_json(&work.join("package-lock.json"));
    let mut all: Vec<PathBuf> = preferred.iter().map(|p| p.join("node_modules")).collect();
    all.extend(candidates(pool));
    if let Some(nm) = all.iter().find(|nm| matches(nm, &pkg, lock.as_ref())) {
        link_node_modules(nm, work)?;
        return Ok(Deps::Linked(nm.clone()));
    }
    if lock.is_none() {
        return Ok(Deps::Unavailable("no lockfile and no matching node_modules".into()));
    }
    // Offline install from the user's npm cache (read-only in the sandbox).
    let cache = capture::fsx::home_dir().map(|h| h.join(".npm")).unwrap_or_default();
    let r = run(
        sb,
        work,
        &["npm", "ci", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", "--cache", &cache.to_string_lossy()],
        &[],
        Duration::from_secs(300),
    )?;
    if !r.ok() {
        let _ = fs::remove_dir_all(work.join("node_modules"));
        let why = r.stderr.lines().rev().find(|l| l.contains("ERR")).unwrap_or("npm ci --offline failed").to_string();
        return Ok(Deps::Unavailable(format!("offline install failed: {why}")));
    }
    // Native packages weren't built (scripts off): swap in a built copy of
    // the same version from the pool.
    let nm = work.join("node_modules");
    let mut native_from = Vec::new();
    for (name, version) in lock.as_ref().map(locked).unwrap_or_default() {
        let dir = nm.join(&name);
        if !is_native(&dir) || is_built(&dir) {
            continue;
        }
        let donor = all.iter().map(|c| c.join(&name)).find(|d| is_built(d) && read_json(&d.join("package.json")).and_then(|p| p.get("version")?.as_str().map(str::to_string)).as_deref() == Some(version.as_str()));
        match donor {
            Some(d) => {
                fs::remove_dir_all(&dir)?;
                crate::link_dir(&d, &dir)?;
                native_from.push(d);
            }
            None => return Ok(Deps::Unavailable(format!("native package {name}@{version} needs a build and no built copy is on this machine"))),
        }
    }
    Ok(Deps::Offline { native_from })
}

/// The project's own test command, or None if it has no tests to run.
pub fn test_command(work: &Path) -> Option<Vec<String>> {
    let pkg = read_json(&work.join("package.json"))?;
    let script = pkg.get("scripts").and_then(|s| s.get("test")).and_then(Value::as_str).unwrap_or("");
    if !script.is_empty() && !script.contains("no test specified") {
        return Some(vec!["npm".into(), "test".into(), "--silent".into()]);
    }
    let deps = declared(&pkg);
    if deps.iter().any(|d| d == "vitest") {
        return Some(vec!["node".into(), "node_modules/vitest/vitest.mjs".into(), "run".into()]);
    }
    if deps.iter().any(|d| d == "jest") {
        return Some(vec!["node".into(), "node_modules/jest/bin/jest.js".into()]);
    }
    None
}

/// Pass/fail counts parsed from a test run's output.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    pub passed: u64,
    pub failed: u64,
}

fn ansi() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*m").expect("regex"))
}

/// Vitest ("Tests  2 failed | 25 passed (27)"), Jest ("Tests:  1 failed,
/// 2 passed, 3 total") and node --test ("# pass 9" / "# fail 1").
pub fn parse_summary(output: &str) -> Option<Summary> {
    static VITEST: OnceLock<Regex> = OnceLock::new();
    static JEST: OnceLock<Regex> = OnceLock::new();
    static NODE: OnceLock<Regex> = OnceLock::new();
    let text = ansi().replace_all(output, "");
    let count = |line: &str, word: &str| -> u64 {
        Regex::new(&format!(r"(\d+)\s+{word}")).ok().and_then(|r| r.captures(line)).and_then(|c| c[1].parse().ok()).unwrap_or(0)
    };
    let vitest = VITEST.get_or_init(|| Regex::new(r"(?m)^\s*Tests\s+(.*\d+\s+(?:passed|failed).*)$").expect("regex"));
    if let Some(c) = vitest.captures_iter(&text).last() {
        return Some(Summary { passed: count(&c[1], "passed"), failed: count(&c[1], "failed") });
    }
    let jest = JEST.get_or_init(|| Regex::new(r"(?m)^Tests:\s+(.*total)").expect("regex"));
    if let Some(c) = jest.captures_iter(&text).last() {
        return Some(Summary { passed: count(&c[1], "passed"), failed: count(&c[1], "failed") });
    }
    let node = NODE.get_or_init(|| Regex::new(r"(?m)^[#ℹ]\s*(pass|fail)\s+(\d+)").expect("regex"));
    let mut s = Summary::default();
    let mut seen = false;
    for c in node.captures_iter(&text) {
        seen = true;
        let n: u64 = c[2].parse().unwrap_or(0);
        if &c[1] == "pass" { s.passed = n } else { s.failed = n }
    }
    seen.then_some(s)
}

/// One run of the project's test command.
#[derive(Debug, Clone)]
pub struct TestRun {
    pub command: Vec<String>,
    pub result: RunResult,
    pub summary: Option<Summary>,
}

/// Run the project's tests in the sandbox (loopback on: test servers).
/// `None` when the project has no tests to run.
pub fn run_tests(sb: &Sandbox, work: &Path, timeout: Duration) -> Result<Option<TestRun>> {
    let Some(command) = test_command(work) else { return Ok(None) };
    let argv: Vec<&str> = command.iter().map(String::as_str).collect();
    let result = run(sb, work, &argv, &[], timeout)?;
    let summary = parse_summary(&format!("{}\n{}", result.stdout, result.stderr));
    Ok(Some(TestRun { command, result, summary }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries() {
        assert_eq!(parse_summary("\x1b[2m      Tests \x1b[22m \x1b[1m\x1b[31m3 failed\x1b[39m | 24 passed (27)"), Some(Summary { passed: 24, failed: 3 }));
        assert_eq!(parse_summary(" Test Files  2 passed (2)\n      Tests  27 passed (27)"), Some(Summary { passed: 27, failed: 0 }));
        assert_eq!(parse_summary("Tests:       1 failed, 2 passed, 3 total"), Some(Summary { passed: 2, failed: 1 }));
        assert_eq!(parse_summary("# tests 10\n# pass 9\n# fail 1"), Some(Summary { passed: 9, failed: 1 }));
        assert_eq!(parse_summary("sh: vitest: command not found"), None);
    }

    #[test]
    fn lockfile_versions_decide_a_match() {
        let tmp = tempfile::tempdir().unwrap();
        let nm = tmp.path().join("node_modules");
        fs::create_dir_all(nm.join("express")).unwrap();
        fs::write(nm.join("express/package.json"), r#"{"version":"4.21.2"}"#).unwrap();
        let pkg = json!({"dependencies": {"express": "^4.21.2"}});
        let lock = |v: &str| json!({"packages": {"": {}, "node_modules/express": {"version": v}}});
        assert!(matches(&nm, &pkg, Some(&lock("4.21.2"))));
        assert!(!matches(&nm, &pkg, Some(&lock("4.22.0"))));
        assert!(matches(&nm, &pkg, None));
    }
}
