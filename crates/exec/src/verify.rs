//! Phase 4b.2: probes P0–P2 and the execution record.
//!
//! - P0 baseline: the test suite at the session's first human prompt
//! - P1 final: the test suite on the final code, run twice (`flaky` if the
//!   two runs disagree)
//! - P2 build: `npm run build` (if the project has one) and `tsc --noEmit`
//!   (if it has a tsconfig and TypeScript), at both points
//!
//! The record is written next to the recording (`exec/record.json`) and read
//! by `tattle check`, which stays deterministic: same recording + same record =
//! same report.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use serde_json::{Value, json};

use crate::workspace::{Deps, TestRun, materialise, resolve_dependencies, run_tests};
use crate::{RunResult, Sandbox, run};

/// Where to look for dependencies.
#[derive(Debug, Clone, Default)]
pub struct DepSources {
    pub project: Vec<PathBuf>,
    pub pool: Vec<PathBuf>,
    /// The Linux dependency pool (Docker backend).
    pub linux: crate::linuxdeps::Options,
}

/// Outcome of a probe, as stored in the record.
fn outcome_of_tests(runs: &[Option<TestRun>]) -> Value {
    let one = |t: &Option<TestRun>| -> Value {
        match t {
            None => json!({"result": "no-tests"}),
            Some(t) => json!({
                "exit": t.result.exit,
                "timed_out": t.result.timed_out,
                "passed": t.summary.as_ref().map(|s| s.passed),
                "failed": t.summary.as_ref().map(|s| s.failed),
                "failing": failing_names(&format!("{}\n{}", t.result.stdout, t.result.stderr)),
                "tail": tail(&format!("{}\n{}", t.result.stdout, t.result.stderr), 800),
            }),
        }
    };
    let list: Vec<Value> = runs.iter().map(one).collect();
    let key = |v: &Value| (v["result"].clone(), v["exit"].clone(), v["passed"].clone(), v["failed"].clone(), v["timed_out"].clone());
    let flaky = list.windows(2).any(|w| key(&w[0]) != key(&w[1]));
    let first = &list[0];
    let outcome = if flaky {
        "flaky"
    } else if first["result"] == "no-tests" {
        "no-tests"
    } else if first["timed_out"] == true {
        "timeout"
    } else if first["failed"].is_null() {
        if first["exit"] == 0 { "passed" } else { "broken" }
    } else if first["failed"].as_u64().unwrap_or(0) > 0 || first["exit"] != 0 {
        "failed"
    } else {
        "passed"
    };
    json!({"outcome": outcome, "runs": list})
}

/// The start and the end of a long log: a crash's error line is usually near
/// the start, its stack trace fills the rest.
fn head_tail(s: &str, n: usize) -> String {
    if s.len() <= 2 * n {
        return s.to_string();
    }
    let mut h = n;
    while !s.is_char_boundary(h) {
        h -= 1;
    }
    format!("{}\n…\n{}", &s[..h], tail(s, n))
}

fn tail(s: &str, n: usize) -> String {
    let start = s.len().saturating_sub(n);
    let mut i = start;
    while !s.is_char_boundary(i) {
        i += 1;
    }
    s[i..].to_string()
}

/// Names of failing tests from vitest (` FAIL  file > suite > name`, `×`),
/// jest (`● suite › name`) or node --test (`not ok N - name`). Best effort,
/// for messages only.
pub fn failing_names(output: &str) -> Vec<String> {
    let text = regex::Regex::new(r"\x1b\[[0-9;]*m").expect("regex").replace_all(output, "");
    let re = regex::Regex::new(r"(?m)^\s*(?:FAIL\s+(\S.*?)\s*$|×\s+(\S.*?)(?:\s+\d+ms)?\s*$|●\s+(\S.*?)\s*$|not ok \d+ - (\S.*?)\s*$)").expect("regex");
    let mut out: Vec<String> = re
        .captures_iter(&text)
        .filter_map(|c| c.get(1).or(c.get(2)).or(c.get(3)).or(c.get(4)).map(|m| m.as_str().trim().to_string()))
        .filter(|n| !n.is_empty() && !n.starts_with("Test Files"))
        .collect();
    out.sort();
    out.dedup();
    out.truncate(20);
    out
}

/// Does the project start an HTTP server? A `start` script, and code that
/// listens (`.listen(`, `createServer`, `express()`, `fastify(`, `Hono`).
pub fn is_server(work: &Path) -> bool {
    let pkg: Value = std::fs::read_to_string(work.join("package.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    if !pkg["scripts"]["start"].is_string() {
        return false;
    }
    let re = regex::Regex::new(r"\.listen\(|createServer\(|\bexpress\(\)|\bfastify\(|new Hono\b").expect("regex");
    walk_code(work).iter().any(|f| std::fs::read_to_string(f).is_ok_and(|t| re.is_match(&t)))
}

/// Source files (not node_modules, dist or tests), for cheap scans.
fn walk_code(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return out };
    for e in entries.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || ["node_modules", "dist", "build", "coverage"].contains(&name.as_str()) {
            continue;
        }
        if p.is_dir() {
            out.extend(walk_code(&p));
        } else if [".js", ".mjs", ".cjs", ".ts", ".mts"].iter().any(|x| name.ends_with(x)) && !name.contains(".test.") && !name.contains(".spec.") {
            out.push(p);
        }
    }
    out
}

const PROBE_JS: &str = r#"
const [port, path, waitMs] = process.argv.slice(2);
const deadline = Date.now() + Number(waitMs);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
(async () => {
  while (Date.now() < deadline) {
    try {
      const r = await fetch(`http://127.0.0.1:${port}${path}`, { redirect: "manual" });
      const body = (await r.text()).slice(0, 200);
      console.log(JSON.stringify({ listening: true, status: r.status, body }));
      return;
    } catch { await sleep(250); }
  }
  console.log(JSON.stringify({ listening: false }));
})();
"#;

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(38123)
}

/// P3: start the app with `npm start` on a free port and a fresh DATA_DIR,
/// then call `/health` (if the code names it) or `/`. Twice; `flaky` when
/// the runs disagree. `None` when the project isn't a server.
pub fn app_probe(sb: &Sandbox, work: &Path) -> Result<Option<Value>> {
    if !is_server(work) {
        return Ok(None);
    }
    let path = if walk_code(work).iter().any(|f| std::fs::read_to_string(f).is_ok_and(|t| t.contains("'/health'") || t.contains("\"/health\"") || t.contains("`/health`"))) { "/health" } else { "/" };
    std::fs::write(sb.tmp.join("cv-app-probe.mjs"), PROBE_JS)?;
    let once = || -> Result<Value> {
        let port = free_port();
        let data = sb.tmp.join(format!("data-{port}"));
        std::fs::create_dir_all(&data)?;
        let log = sb.tmp.join(format!("server-{port}.log"));
        let script = format!(
            // A fresh TMPDIR per run: tsx leaves an IPC pipe behind that the
            // second run collided with (EADDRINUSE, seen as flaky on R46).
            "mkdir -p '{rtmp}'; (TMPDIR='{rtmp}' PORT={port} DATA_DIR='{data}' npm start --silent > '{log}' 2>&1) & SPID=$!; node '{probe}' {port} {path} 20000; if kill -0 $SPID 2>/dev/null; then echo ALIVE; else echo EXITED; fi",
            rtmp = sb.tmp.join(format!("run-{port}")).display(),
            data = data.display(),
            log = log.display(),
            probe = sb.tmp.join("cv-app-probe.mjs").display(),
        );
        let r: RunResult = run(sb, work, &["/bin/sh", "-c", &script], &[], Duration::from_secs(45))?;
        let probe: Value = r.stdout.lines().find_map(|l| serde_json::from_str(l).ok()).unwrap_or(json!({"listening": false}));
        let alive = r.stdout.contains("ALIVE");
        let mut server_log = std::fs::read_to_string(&log).unwrap_or_default();
        // Temp paths differ on every run; show them relative to the project.
        for w in [work.to_path_buf(), capture::fsx::canonical(work).unwrap_or_else(|_| work.to_path_buf())] {
            server_log = server_log.replace(&w.display().to_string(), "<project>");
        }
        let server_log = head_tail(&server_log, 800);
        // A permission error from the sandbox is our environment, not the app.
        let env = regex::Regex::new(r"(?i)\bEPERM\b|operation not permitted").expect("regex");
        let outcome = match (probe["listening"].as_bool(), probe["status"].as_u64()) {
            _ if env.is_match(&server_log) => "couldnt-run",
            (Some(true), Some(s)) if s < 500 => "started",
            (Some(true), _) => "error-response",
            _ if !alive => "crashed",
            _ => "no-listen",
        };
        Ok(json!({"outcome": outcome, "path": path, "status": probe["status"], "server_log": server_log}))
    };
    let a = once()?;
    let b = once()?;
    let outcome = if a["outcome"] != b["outcome"] { "flaky".to_string() } else { a["outcome"].as_str().unwrap_or("").to_string() };
    Ok(Some(json!({"outcome": outcome, "runs": [a, b]})))
}

fn build_probe(sb: &Sandbox, work: &Path) -> Result<Value> {
    let pkg: Value = std::fs::read_to_string(work.join("package.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let mut out = json!({});
    let mut probe = |name: &str, argv: &[&str]| -> Result<()> {
        let a: RunResult = run(sb, work, argv, &[], Duration::from_secs(180))?;
        let b: RunResult = run(sb, work, argv, &[], Duration::from_secs(180))?;
        let outcome = if a.ok() != b.ok() { "flaky" } else if a.timed_out { "timeout" } else if a.ok() { "passed" } else { "failed" };
        out[name] = json!({"outcome": outcome, "command": argv.join(" "), "tail": tail(&format!("{}\n{}", a.stdout, a.stderr), 800)});
        Ok(())
    };
    if pkg["scripts"]["build"].is_string() {
        probe("build", &["npm", "run", "build", "--silent"])?;
    }
    if work.join("tsconfig.json").is_file() && work.join("node_modules/typescript/bin/tsc").exists() {
        probe("typecheck", &["node", "node_modules/typescript/bin/tsc", "--noEmit"])?;
    }
    Ok(out)
}

/// Verify one snapshot: materialise, dependencies, tests (twice), build.
fn probe_snapshot(git_dir: &Path, commit: &str, root: &Path, label: &str, sources: &DepSources, timeout: Duration, twice: bool) -> Result<Value> {
    let work = root.join(label).join("work");
    let tmp = root.join(label).join("tmp");
    std::fs::create_dir_all(&tmp)?;
    materialise(git_dir, commit, &work)?;
    let mut sb = Sandbox { work: work.clone(), tmp, deny_read: vec![], loopback: true, node_modules: None };
    let deps = resolve_dependencies(&work, &sb, &sources.project, &sources.pool, &sources.linux)?;
    if let Deps::Pool(nm) = &deps {
        sb.node_modules = Some(nm.clone());
    }
    let mut v = json!({"commit": commit, "dependencies": deps.to_json()});
    if matches!(deps, Deps::Unavailable(_)) {
        v["tests"] = json!({"outcome": "couldnt-run"});
        return Ok(v);
    }
    // Build first: tests may run the built output (real session H23's CLI
    // tests run dist/cli.js), as the project's own workflow does.
    v["build"] = build_probe(&sb, &work)?;
    if let Some(app) = app_probe(&sb, &work)? {
        v["app"] = app;
    }
    let mut runs = vec![run_tests(&sb, &work, timeout)?];
    if twice {
        runs.push(run_tests(&sb, &work, timeout)?);
    }
    v["tests"] = outcome_of_tests(&runs);
    // An environment failure is "couldn't run", never a test failure.
    let env = regex::Regex::new(r"(?i)\bEPERM\b|operation not permitted|EAI_AGAIN|ENOTFOUND|getaddrinfo").expect("regex");
    if v["tests"]["outcome"] == "broken" && v["tests"]["runs"].as_array().is_some_and(|r| r.iter().any(|x| env.is_match(x["tail"].as_str().unwrap_or("")))) {
        v["tests"]["outcome"] = "couldnt-run".into();
    }
    Ok(v)
}

/// Run P0–P2 for a session and return the execution record.
pub fn verify_session(git_dir: &Path, baseline: Option<&str>, final_commit: &str, sources: &DepSources, timeout: Duration) -> Result<Value> {
    let root = tempfile_dir()?;
    let node = std::process::Command::new("node").arg("--version").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    let mut record = json!({
        "schema": 1,
        "env": env_info(node),
        "final": probe_snapshot(git_dir, final_commit, &root, "final", sources, timeout, true)?,
    });
    if let Some(b) = baseline.filter(|b| *b != final_commit) {
        record["baseline"] = probe_snapshot(git_dir, b, &root, "baseline", sources, timeout, false)?;
        // #1/#2: each side's tests on the other side's code, per test. Only
        // when both suites ran (passed or failed, not broken or missing).
        let ran = |v: &Value| matches!(v["tests"]["outcome"].as_str(), Some("passed" | "failed"));
        if ran(&record["baseline"]) && ran(&record["final"]) {
            let pool = |label: &str| record[label]["dependencies"]["from"].as_str().filter(|_| record[label]["dependencies"]["strategy"] == "linux-pool").map(PathBuf::from);
            let sb = |label: &str| Sandbox { work: root.join(label).join("work"), tmp: root.join(label).join("tmp"), deny_read: vec![], loopback: true, node_modules: pool(label) };
            if let Some(c) = crate::cross::cross(git_dir, b, final_commit, &sb("baseline"), &sb("final"), &root, sources, timeout)? {
                record["cross"] = c;
            }
        }
    }
    let _ = capture::fsx::remove_dir_all(&root);
    Ok(record)
}

/// Which sandbox ran the probes (and, for Docker, which image).
fn env_info(host_node: String) -> serde_json::Value {
    match crate::detect_backend() {
        crate::Backend::Docker => json!({"backend": "docker", "image": crate::docker::image()}),
        _ => json!({"backend": "macos-sandbox", "node": host_node}),
    }
}

fn tempfile_dir() -> Result<PathBuf> {
    // A short path: Unix socket paths are limited to ~104 bytes on macOS, and
    // tools open sockets in TMPDIR (tsx's IPC pipe). Under the long
    // /var/folders/... temp dir the path was truncated and the second app
    // run collided with the first (EADDRINUSE).
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
    let dir = base.join(format!("tattle{}-{}", std::process::id(), nanos % 1_000_000));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failing_test_names() {
        let vitest = " FAIL  tests/settle.test.ts > settleUp > has several debtors pay one creditor\n   × rejects zero 3ms\n";
        assert_eq!(failing_names(vitest), vec!["rejects zero", "tests/settle.test.ts > settleUp > has several debtors pay one creditor"]);
        assert_eq!(failing_names("not ok 3 - parses csv\nok 4 - other"), vec!["parses csv"]);
    }

    #[test]
    fn long_logs_keep_the_error_line() {
        let log = format!("TypeError: db.pragma is not a function\n{}", "    at frame\n".repeat(500));
        let kept = head_tail(&log, 800);
        assert!(kept.starts_with("TypeError: db.pragma"));
        assert!(kept.len() < 1700);
        assert_eq!(head_tail("short", 800), "short");
    }
}
