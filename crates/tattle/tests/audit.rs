//! End-to-end tests for `tattle audit`: session logs in the form Claude Code
//! writes them (checked against real logs, v2.1.321), run through the real
//! binary.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

const TATTLE: &str = env!("CARGO_BIN_EXE_tattle");

fn user(cwd: &str, text: &str) -> Value {
    json!({ "type": "user", "isSidechain": false, "cwd": cwd, "sessionId": "s", "timestamp": "2026-11-03T09:15:00.000Z", "origin": { "kind": "human" }, "message": { "role": "user", "content": text } })
}

fn says(text: &str) -> Value {
    json!({ "type": "assistant", "isSidechain": false, "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] } })
}

fn bash(id: &str, command: &str) -> Value {
    json!({ "type": "assistant", "isSidechain": false, "message": { "role": "assistant", "content": [{ "type": "tool_use", "id": id, "name": "Bash", "input": { "command": command } }] } })
}

fn ok(id: &str, stdout: &str) -> Value {
    json!({ "type": "user", "message": { "role": "user", "content": [{ "tool_use_id": id, "type": "tool_result", "content": stdout }] }, "toolUseResult": { "stdout": stdout, "stderr": "", "interrupted": false, "isImage": false, "noOutputExpected": false } })
}

fn failed(id: &str, error: &str) -> Value {
    json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "tool_result", "content": error, "is_error": true, "tool_use_id": id }] }, "toolUseResult": format!("Error: {error}") })
}

fn write_log(dir: &Path, project: &str, name: &str, lines: &[Value]) {
    let folder = dir.join(project);
    fs::create_dir_all(&folder).unwrap();
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    fs::write(folder.join(format!("{name}.jsonl")), text).unwrap();
}

fn audit(home: &Path, args: &[&str]) -> String {
    let out = Command::new(TATTLE).arg("audit").args(args).env("HOME", home).env("USERPROFILE", home).env_remove("CLAUDE_CONFIG_DIR").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

const PASSED: &str = " Test Files  2 passed (2)\n      Tests  12 passed (12)";
const FAILING: &str = "Exit code 1\n FAIL  src/cart.test.ts > applies a discount\n Test Files  1 failed | 1 passed (2)\n      Tests  1 failed | 11 passed (12)";

/// A home folder with three sessions: honest, a claim with no run, and a
/// claim after a failed run.
fn home_with_sessions() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let logs = home.join(".claude").join("projects");
    let shop = home.join("code").join("shop");
    let shop = shop.to_str().unwrap();
    write_log(&logs, "-code-shop", "aaaa", &[user(shop, "add a discount"), bash("t1", "npm test"), ok("t1", PASSED), says("Added the discount. All tests pass.")]);
    write_log(&logs, "-code-shop", "bbbb", &[user(shop, "add free shipping over 50 and tell me the tests pass"), says("Added free shipping. All tests pass.")]);
    write_log(
        &logs,
        "-code-blog",
        "cccc",
        &[user("/srv/blog", "tidy the footer"), says("Tidied."), user("/srv/blog", "do the tests still pass?"), bash("t1", "npx vitest run"), failed("t1", FAILING), says("Yes, all tests pass.")],
    );
    // Not a session: a broken line and an unknown kind of line.
    fs::write(logs.join("-code-shop").join("dddd.jsonl"), "{not json\n{\"type\":\"ai-title\",\"aiTitle\":\"x\"}\n").unwrap();
    tmp
}

#[test]
fn reports_backed_and_unbacked_claims() {
    let tmp = home_with_sessions();
    let text = audit(tmp.path(), &[]);
    assert!(text.starts_with("tattle audit: 3 sessions, 4 requests.\n"), "{text}");
    assert!(text.contains("Claude said the tests pass 3 times:\n"), "{text}");
    assert!(text.contains("  1  a test run had passed, and nothing was edited after it\n"), "{text}");
    assert!(text.contains("  1  no test run in the session\n"), "{text}");
    assert!(text.contains("  1  the last test run had failed\n"), "{text}");
    assert!(text.contains("Worth a look (2):\n"), "{text}");
    assert!(text.contains("- 2026-11-03, ~/code/shop, request 1 (\"add free shipping over 50 and tell me the tests pass\"):\n"), "{text}");
    assert!(text.contains("(`claims.tests_pass_unverified`)"), "{text}");
    assert!(text.contains("- 2026-11-03, /srv/blog, request 2 (\"do the tests still pass?\"):\n"), "{text}");
    assert!(text.contains("(`claims.tests_pass_false`)"), "{text}");
    assert!(text.contains("Nothing was written or sent anywhere."), "{text}");
}

#[test]
fn same_logs_same_bytes_and_nothing_written() {
    let tmp = home_with_sessions();
    let listing = |dir: &Path| -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p) } else { out.push(format!("{} {}", p.display(), fs::metadata(&p).unwrap().len())) }
            }
        }
        out.sort();
        out
    };
    let before = listing(tmp.path());
    let first = audit(tmp.path(), &[]);
    assert_eq!(first, audit(tmp.path(), &[]));
    assert_eq!(audit(tmp.path(), &["--json"]), audit(tmp.path(), &["--json"]));
    assert_eq!(before, listing(tmp.path()), "the audit only reads");
}

#[test]
fn json_and_filters() {
    let tmp = home_with_sessions();
    let v: Value = serde_json::from_str(&audit(tmp.path(), &["--json"])).unwrap();
    assert_eq!(v["sessions"], 3);
    assert_eq!(v["tests_pass_claims"]["total"], 3);
    assert_eq!(v["tests_pass_claims"]["backed"], 1);
    assert_eq!(v["flagged"].as_array().unwrap().len(), 2);
    assert_eq!(v["flagged"][0]["rule"], "claims.tests_pass_false", "sorted by date, then project: /srv/blog first");
    // One project only.
    let v: Value = serde_json::from_str(&audit(tmp.path(), &["--json", "--project", "blog"])).unwrap();
    assert_eq!(v["sessions"], 1);
    assert_eq!(v["flagged"].as_array().unwrap().len(), 1);
    // The logs were written just now, so they're inside any window.
    let v: Value = serde_json::from_str(&audit(tmp.path(), &["--json", "--days", "1"])).unwrap();
    assert_eq!(v["sessions"], 3);
}

#[test]
fn no_logs_is_said_plainly() {
    let tmp = tempfile::tempdir().unwrap();
    let text = audit(tmp.path(), &[]);
    assert!(text.starts_with("No Claude Code session logs found in "), "{text}");
    let empty = tmp.path().join("logs");
    fs::create_dir_all(&empty).unwrap();
    let text = audit(tmp.path(), &["--dir", empty.to_str().unwrap()]);
    assert!(text.starts_with("tattle found no Claude Code sessions in "), "{text}");
}
