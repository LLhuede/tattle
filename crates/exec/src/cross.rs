//! Cross runs (improvement plan #1 and #2): each side's tests against the
//! other side's code, with per-test results.
//!
//! - `new_tests_old_code` (P5): the final test files on the code from the
//!   start of the session. A test that passes here doesn't show the change:
//!   it passed before it too.
//! - `old_tests_new_code` (P4): the starting test files on the final code.
//!   A test that passed at the start and fails here pinned behaviour the
//!   session changed.
//!
//! Per-test results come from vitest's JSON reporter. Other runners give no
//! cross record. The mixed copies run twice; a test whose runs disagree is
//! `flaky` and never used as evidence.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::workspace::{Deps, materialise, resolve_dependencies};
use crate::{Sandbox, run};

use crate::verify::DepSources;

/// Test files: anything under a `test`, `tests` or `__tests__` folder, and
/// `*.test.*` / `*.spec.*` files.
pub fn is_test_path(p: &str) -> bool {
    let p = p.replace('\\', "/");
    let in_dir = p.split('/').rev().skip(1).any(|d| matches!(d, "test" | "tests" | "__tests__"));
    let name = p.rsplit('/').next().unwrap_or("");
    let file = [".test.", ".spec."].iter().any(|m| name.contains(m))
        && [".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".mts", ".cts"].iter().any(|e| name.ends_with(e));
    in_dir || file
}

fn git(git_dir: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git").arg("--git-dir").arg(git_dir).args(args).output().context("git")?;
    anyhow::ensure!(out.status.success(), "git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr));
    Ok(out.stdout)
}

fn files_at(git_dir: &Path, commit: &str) -> Result<Vec<String>> {
    Ok(String::from_utf8_lossy(&git(git_dir, &["ls-tree", "-r", "--name-only", commit])?).lines().map(str::to_string).collect())
}

/// Test files that differ between two commits (added, removed or edited).
pub fn changed_test_files(git_dir: &Path, a: &str, b: &str) -> Result<Vec<String>> {
    let out = git(git_dir, &["diff", "--name-only", "--no-renames", a, b])?;
    Ok(String::from_utf8_lossy(&out).lines().filter(|p| is_test_path(p)).map(str::to_string).collect())
}

/// `code` commit's files with `tests` commit's test files swapped in.
fn hybrid(git_dir: &Path, code: &str, tests: &str, dest: &Path) -> Result<()> {
    materialise(git_dir, code, dest)?;
    for f in files_at(git_dir, code)?.iter().filter(|f| is_test_path(f)) {
        let _ = std::fs::remove_file(dest.join(f));
    }
    for f in files_at(git_dir, tests)?.iter().filter(|f| is_test_path(f)) {
        let body = git(git_dir, &["show", &format!("{tests}:{f}")])?;
        let path = dest.join(f);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, body)?;
    }
    Ok(())
}

/// The vitest command with the JSON reporter, or None if the project's tests
/// don't run on vitest.
fn json_command(work: &Path, out: &Path) -> Option<Vec<String>> {
    let pkg: Value = serde_json::from_str(&std::fs::read_to_string(work.join("package.json")).ok()?).ok()?;
    let script = pkg["scripts"]["test"].as_str().unwrap_or("");
    let reporter = ["--reporter=json".to_string(), format!("--outputFile={}", out.display())];
    if script.contains("vitest") {
        // npm passes extra arguments on to the script's last command.
        let mut v: Vec<String> = ["npm", "test", "--silent", "--"].iter().map(|s| s.to_string()).collect();
        v.extend(reporter);
        return Some(v);
    }
    if script.is_empty() && work.join("node_modules/vitest/vitest.mjs").exists() {
        let mut v: Vec<String> = ["node", "node_modules/vitest/vitest.mjs", "run"].iter().map(|s| s.to_string()).collect();
        v.extend(reporter);
        return Some(v);
    }
    None
}

/// Per-test results ("file > describe > title" → passed / failed / skipped)
/// from one run, or None if the runner isn't vitest or wrote no report.
pub fn per_test(sb: &Sandbox, work: &Path, timeout: Duration, n: usize) -> Result<Option<BTreeMap<String, String>>> {
    let out = sb.tmp.join(format!("vitest-{n}.json"));
    let _ = std::fs::remove_file(&out);
    let Some(argv) = json_command(work, &out) else { return Ok(None) };
    let argv: Vec<&str> = argv.iter().map(String::as_str).collect();
    let r = run(sb, work, &argv, &[], timeout)?;
    if r.timed_out {
        return Ok(None);
    }
    let Ok(text) = std::fs::read_to_string(&out) else { return Ok(None) };
    let Ok(report) = serde_json::from_str::<Value>(&text) else { return Ok(None) };
    Ok(Some(parse_report(&report, work)))
}

pub fn parse_report(report: &Value, work: &Path) -> BTreeMap<String, String> {
    let roots: Vec<String> = [Some(work.to_path_buf()), capture::fsx::canonical(work).ok()]
        .into_iter()
        .flatten()
        .map(|p| format!("{}/", p.display()))
        .collect();
    let mut map = BTreeMap::new();
    for file in report["testResults"].as_array().into_iter().flatten() {
        let mut name = file["name"].as_str().unwrap_or("").to_string();
        for r in &roots {
            if let Some(rest) = name.strip_prefix(r.as_str()) {
                name = rest.to_string();
            }
        }
        let results = file["assertionResults"].as_array().cloned().unwrap_or_default();
        if results.is_empty() && file["status"] == "failed" {
            // The file didn't load (syntax error, missing import).
            map.insert(format!("{name} > (file failed to load)"), "failed".to_string());
        }
        for t in results {
            let mut parts: Vec<String> = vec![name.clone()];
            parts.extend(t["ancestorTitles"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string));
            parts.push(t["title"].as_str().unwrap_or("").to_string());
            let status = match t["status"].as_str() {
                Some("passed") => "passed",
                Some("failed") => "failed",
                _ => "skipped",
            };
            map.insert(parts.join(" > "), status.to_string());
        }
    }
    map
}

/// Two runs merged: a test whose runs disagree is `flaky`.
fn merge(a: BTreeMap<String, String>, b: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    a.into_iter()
        .map(|(k, v)| {
            let same = b.get(&k).is_some_and(|w| *w == v);
            (k, if same { v } else { "flaky".to_string() })
        })
        .collect()
}

/// Build (if the project has a build script) so tests that run built output
/// work, as in the normal probes. Failures are ignored here.
fn build(sb: &Sandbox, work: &Path) {
    let has_build = std::fs::read_to_string(work.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .is_some_and(|p| p["scripts"]["build"].is_string());
    if has_build {
        let _ = run(sb, work, &["npm", "run", "build", "--silent"], &[], Duration::from_secs(180));
    }
}

/// One side's tests against the other side's code, twice.
fn mixed(git_dir: &Path, code: &str, tests: &str, dir: &Path, sources: &DepSources, timeout: Duration) -> Result<Option<BTreeMap<String, String>>> {
    let work = dir.join("work");
    let tmp = dir.join("tmp");
    std::fs::create_dir_all(&tmp)?;
    hybrid(git_dir, code, tests, &work)?;
    let mut sb = Sandbox { work: work.clone(), tmp, deny_read: vec![], loopback: true, node_modules: None };
    match resolve_dependencies(&work, &sb, &sources.project, &sources.pool, &sources.linux)? {
        Deps::Unavailable(_) => return Ok(None),
        Deps::Pool(nm) => sb.node_modules = Some(nm),
        _ => {}
    }
    build(&sb, &work);
    let Some(a) = per_test(&sb, &work, timeout, 1)? else { return Ok(None) };
    let Some(b) = per_test(&sb, &work, timeout, 2)? else { return Ok(None) };
    Ok(Some(merge(a, &b)))
}

/// The cross record, or None when per-test results aren't available.
/// `baseline_work` and `final_work` are the copies the normal probes used
/// (dependencies installed, already built).
#[allow(clippy::too_many_arguments)]
pub fn cross(
    git_dir: &Path,
    baseline: &str,
    final_commit: &str,
    baseline_sb: &Sandbox,
    final_sb: &Sandbox,
    root: &Path,
    sources: &DepSources,
    timeout: Duration,
) -> Result<Option<Value>> {
    let Some(base) = per_test(baseline_sb, &baseline_sb.work, timeout, 1)? else { return Ok(None) };
    let Some(fin) = per_test(final_sb, &final_sb.work, timeout, 1)? else { return Ok(None) };
    let new_on_old = mixed(git_dir, baseline, final_commit, &root.join("p5"), sources, timeout)?;
    let old_on_new = mixed(git_dir, final_commit, baseline, &root.join("p4"), sources, timeout)?;
    Ok(Some(json!({
        "runner": "vitest-json",
        "baseline": base,
        "final": fin,
        "new_tests_old_code": new_on_old,
        "old_tests_new_code": old_on_new,
        "tests_changed": changed_test_files(git_dir, baseline, final_commit)?,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_paths() {
        assert!(is_test_path("tests/api.test.ts"));
        assert!(is_test_path("src/streak.test.js"));
        assert!(is_test_path("test/helpers.ts"));
        assert!(is_test_path("src/__tests__/a.ts"));
        assert!(is_test_path("src/a.spec.tsx"));
        assert!(!is_test_path("src/testing.ts"));
        assert!(!is_test_path("src/latest.ts"));
        assert!(!is_test_path("tests"));
    }

    #[test]
    fn report_parsing() {
        let report = json!({"testResults": [
            {"name": "/tmp/w/tests/a.test.ts", "status": "passed", "assertionResults": [
                {"ancestorTitles": ["settle"], "title": "splits evenly", "status": "passed"},
                {"ancestorTitles": [], "title": "later", "status": "todo"}]},
            {"name": "/tmp/w/tests/b.test.ts", "status": "failed", "assertionResults": []}]});
        let m = parse_report(&report, Path::new("/tmp/w"));
        assert_eq!(m["tests/a.test.ts > settle > splits evenly"], "passed");
        assert_eq!(m["tests/a.test.ts > later"], "skipped");
        assert_eq!(m["tests/b.test.ts > (file failed to load)"], "failed");
    }

    #[test]
    fn disagreeing_runs_are_flaky() {
        let a = BTreeMap::from([("x".to_string(), "passed".to_string()), ("y".to_string(), "failed".to_string())]);
        let b = BTreeMap::from([("x".to_string(), "failed".to_string()), ("y".to_string(), "failed".to_string())]);
        let m = merge(a, &b);
        assert_eq!(m["x"], "flaky");
        assert_eq!(m["y"], "failed");
    }
}
