//! Phase 2 adapter: turns recorded Claude Code hook events into typed
//! actions the checks can reason about.
//!
//! Everything agent-specific lives here, so supporting another agent means
//! writing another adapter, not changing the checks.
//!
//! Unknowns stay unknown: if a test run's result can't be determined, its
//! outcome is `None` and checks that need it stay silent (precision over
//! recall).
//!
//! Claude Code facts this relies on (hooks reference, checked against real
//! recordings): `PostToolUse` fires only when a tool succeeded, so a Bash
//! command there exited 0. A failed tool fires `PostToolUseFailure`
//! instead, with an `error` whose first line is `Exit code N` for commands
//! that ran. Bash responses carry no exit code. `Stop` carries the agent's
//! final message as `last_assistant_message`.

use std::sync::OnceLock;

use regex::Regex;
use serde_json::Value;

/// Tools that write files.
pub const EDIT_TOOLS: &[&str] = &["Edit", "Write", "MultiEdit", "NotebookEdit"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    /// `true` once the tool has finished (PostToolUse or
    /// PostToolUseFailure), `false` for PreToolUse.
    pub post: bool,
    /// The tool call failed (its PreToolUse is marked too, matched by
    /// `tool_use_id`). A failed edit changed nothing.
    pub failed: bool,
    pub name: String,
    /// Project-relative path for file tools. `None` if outside the project.
    pub path: Option<String>,
    /// The path as recorded (with `<PROJECT>` / `<HOME>` placeholders).
    pub raw_path: Option<String>,
    /// Shell command for `Bash`.
    pub command: Option<String>,
    /// Command result, when known (PostToolUse only).
    pub outcome: Option<Outcome>,
    /// For `Edit`: the new text keeps the old text whole (it only adds).
    /// `false` when unknown (e.g. a `Write`, or text trimmed by the recorder).
    pub additive: bool,
    /// For `Edit`: the text replaced and its replacement; for `Write`: the
    /// new contents (`edit_old` is `None`). `None` when the recorder trimmed
    /// the text.
    pub edit_old: Option<String>,
    pub edit_new: Option<String>,
    /// For `Bash`: the output carries a test runner's own summary ("Ran 26
    /// tests", "# pass 9", "test result: ok"), whatever the command looked
    /// like.
    pub runner_output: bool,
}

impl Tool {
    /// A file-writing tool call that didn't fail.
    pub fn is_edit(&self) -> bool {
        EDIT_TOOLS.contains(&self.name.as_str()) && !self.failed
    }

    /// A `Bash` call that ran tests: the command is a known test command,
    /// or its output shows a test runner's summary (everyday batches 1–4:
    /// most false alerts came from command shapes the patterns missed).
    pub fn is_test_run(&self) -> bool {
        self.name == "Bash" && (self.runner_output || self.command.as_deref().is_some_and(is_test_command))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Index into `Recording::events`.
    pub index: usize,
    pub seq: Option<u64>,
    /// Hook event name, e.g. `PostToolUse`.
    pub name: String,
    pub tool: Option<Tool>,
    /// The recorder couldn't capture this event fully.
    pub capture_problem: Option<String>,
    /// The agent's final message, on `Stop` events.
    pub message: Option<String>,
    /// Links a tool call's PreToolUse to its result event.
    pub tool_use_id: Option<String>,
}

/// Tool results recorded in the transcript: `tool_use_id` → is it an
/// error? A second source for calls whose hook result is missing (e.g.
/// headless permission denials fire no hook).
pub fn transcript_results(transcript: &[Value]) -> std::collections::BTreeMap<String, bool> {
    transcript
        .iter()
        .filter_map(|line| line["message"]["content"].as_array())
        .flatten()
        .filter(|b| b["type"] == "tool_result")
        .filter_map(|b| Some((b["tool_use_id"].as_str()?.to_string(), b["is_error"] == true)))
        .collect()
}

pub fn normalise(events: &[Value], transcript: &[Value]) -> Vec<Event> {
    // Tool calls that failed or were denied: from hooks, or from the
    // transcript when no hook reported the result.
    let from_transcript = transcript_results(transcript);
    let failed_ids: std::collections::BTreeSet<&str> = events
        .iter()
        .filter(|e| e["event"] == "PostToolUseFailure" || e["event"] == "PermissionDenied")
        .filter_map(|e| e["payload"]["tool_use_id"].as_str())
        .chain(from_transcript.iter().filter(|(_, err)| **err).map(|(id, _)| id.as_str()))
        .collect();
    events
        .iter()
        .enumerate()
        .map(|(index, e)| {
            let name = e["event"].as_str().unwrap_or("Unknown").to_string();
            let payload = &e["payload"];
            let tool = payload["tool_name"].as_str().map(|tool_name| {
                let input = &payload["tool_input"];
                let raw_path = input["file_path"]
                    .as_str()
                    .or_else(|| input["notebook_path"].as_str())
                    .or_else(|| input["path"].as_str()) // Grep / Glob
                    .map(str::to_string);
                let path = raw_path.as_deref().and_then(project_path);
                let command = input["command"].as_str().map(str::to_string);
                let is_failure_event = name == "PostToolUseFailure";
                let post = name == "PostToolUse" || is_failure_event;
                let failed = is_failure_event
                    || name == "PermissionDenied"
                    || payload["tool_use_id"].as_str().is_some_and(|id| failed_ids.contains(id));
                let output = if is_failure_event {
                    payload["error"].as_str().unwrap_or("").to_string()
                } else {
                    let r = &payload["tool_response"];
                    format!("{}\n{}", r["stdout"].as_str().unwrap_or(""), r["stderr"].as_str().unwrap_or(""))
                };
                let runner_output = tool_name == "Bash" && post && shows_runner_summary(&output) && !command.as_deref().is_some_and(only_reads);
                let is_test = runner_output || command.as_deref().is_some_and(is_test_command);
                let outcome = match (post, tool_name == "Bash", is_failure_event) {
                    (true, true, true) if is_test => test_failure_outcome(payload, command.as_deref().unwrap_or("")),
                    (true, true, true) => failure_outcome(payload),
                    (true, true, false) => command_outcome(&payload["tool_response"], is_test),
                    _ => None,
                };
                let additive = match (input["old_string"].as_str(), input["new_string"].as_str()) {
                    (Some(old), Some(new)) => !old.contains("cv-trimmed") && !new.contains("cv-trimmed") && new.contains(old),
                    _ => false,
                };
                let untrimmed = |v: &Value| v.as_str().filter(|t| !t.contains("cv-trimmed")).map(str::to_string);
                let edit_old = untrimmed(&input["old_string"]);
                let edit_new = untrimmed(&input["new_string"]).or_else(|| untrimmed(&input["content"]));
                Tool { post, failed, name: tool_name.to_string(), path, raw_path, command, outcome, additive, edit_old, edit_new, runner_output }
            });
            let capture_problem = ["payload_error", "snapshot_error", "transcript_error"]
                .iter()
                .find_map(|k| e[*k].as_str().map(|v| format!("{k}: {v}")));
            let message = (name == "Stop")
                .then(|| payload["last_assistant_message"].as_str().map(str::to_string))
                .flatten();
            let tool_use_id = payload["tool_use_id"].as_str().map(str::to_string);
            Event { index, seq: e["seq"].as_u64(), name, tool, capture_problem, message, tool_use_id }
        })
        .collect()
}

fn exit_line_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\s*Exit code (-?\d+)").expect("regex"))
}

/// Result of a Bash call that fired `PostToolUseFailure`: the exit code
/// from the error's first line. Interrupted or unclear failures (timeouts)
/// give `None`.
pub fn failure_outcome(payload: &Value) -> Option<Outcome> {
    if payload["is_interrupt"] == true {
        return None;
    }
    let first = payload["error"].as_str()?.lines().next()?;
    let code: i64 = exit_line_re().captures(first)?[1].parse().ok()?;
    Some(if code == 0 { Outcome::Passed } else { Outcome::Failed })
}

/// `<PROJECT>/src/a.ts` → `src/a.ts`. Paths outside the project → `None`.
pub fn project_path(p: &str) -> Option<String> {
    let rest = p.strip_prefix("<PROJECT>")?;
    let rest = rest.trim_start_matches('/');
    (!rest.is_empty()).then(|| rest.to_string())
}

fn test_command_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
            (?:^|[\s;&|(])
            (?:
                (?:npx|bunx|pnpm\s+exec|pnpm\s+dlx|yarn)\s+(?:vitest|jest)\b
              | (?:vitest|jest)(?:\s|$)
              | (?:npm|pnpm|yarn|bun)\s+(?:run\s+)?test(?::\S+)?\b
              | npm\s+t\b
              | cargo\s+(?:\+\S+\s+)?(?:test|nextest\s+run)\b
            )",
        )
        .expect("regex")
    })
}

/// Runs the project's tests: Vitest, Jest, a package `test` script, `cargo
/// test`, or another common runner run directly (`node --test`, pytest, `go
/// test`, mocha, rspec, …).
pub fn is_test_command(cmd: &str) -> bool {
    // Text a heredoc writes to a file isn't run (a README listing `go test`).
    let cmd = crate::behaviour::strip_heredoc_text(cmd);
    test_command_re().is_match(&cmd) || other_runner_re().is_match(&cmd)
}

/// Output with a test runner's own summary line. Anchored at line starts,
/// so `grep` output (`file:12:Ran 3 tests`) doesn't count.
pub fn shows_runner_summary(output: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?mx)
            ^Ran\s+\d+\s+tests?\s+in\s+[\d.]+s\s*$                    # unittest
          | ^\#\s+tests\s+\d+\s*$                                     # node --test (TAP)
          | ^\s*Test\s+Files\s+\d+\s+(?:passed|failed)                  # vitest
          | ^\s*Tests:?\s+\d+\s+(?:passed|failed)                        # vitest / jest
          | ^test\s+result:\s+(?:ok|FAILED)\.\s+\d+\s+passed            # cargo
          | ^=+\s.*\b\d+\s+(?:passed|failed)\b.*\bin\s+[\d.]+s\b        # pytest
          | ^(?:ok|FAIL)\s+\S+\s+(?:[\d.]+s|\(cached\))\s*$            # go test
          | ^\s+\d+\s+passing\s+\(\d+                                   # mocha
          | ^\d+\s+runs?,\s+\d+\s+assertions?,\s+\d+\s+failures?,\s+\d+\s+errors?  # minitest
          | ^\d+\s+examples?,\s+\d+\s+failures?                          # rspec
            ",
        )
        .expect("regex")
    })
    .is_match(output)
}

/// Every command on the line only shows existing text (`cat old-run.log`,
/// `git show HEAD:ci.log | grep Tests`): runner output it prints is an old
/// run, not one happening now.
fn only_reads(cmd: &str) -> bool {
    let cmd = crate::behaviour::strip_heredoc_text(cmd);
    let readers = ["cat", "head", "tail", "less", "more", "grep", "rg", "sed", "awk", "jq", "bat", "type", "echo", "printf", "tee", "wc", "sort", "uniq", "cut"];
    let first = |seg: &str| seg.split_whitespace().next().map(|w| w.rsplit('/').next().unwrap_or(w).to_string());
    let segments: Vec<&str> = cmd.split(['\n', ';', '|', '&']).filter(|s| !s.trim().is_empty()).collect();
    !segments.is_empty()
        && segments.iter().all(|seg| {
            let words: Vec<&str> = seg.split_whitespace().collect();
            let git_read = words.first() == Some(&"git") && words.get(1).is_some_and(|w| matches!(*w, "show" | "log" | "diff" | "cat-file"));
            git_read || first(seg).is_some_and(|w| readers.contains(&w.as_str()))
        })
}

/// A shell line with more than one command (`a; b`, `a && b`, a newline).
fn is_compound(cmd: &str) -> bool {
    let cmd = crate::behaviour::strip_heredoc_text(cmd);
    cmd.trim().contains(['\n', ';']) || cmd.contains("&&") || cmd.contains("||")
}

/// A test runner's own summary of a passing run.
fn summary_passed_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // unittest "OK"; TAP "# fail 0"; vitest "Tests  3 passed"; jest "Tests: 5
    // passed"; cargo "test result: ok"; go "ok  pkg"; pytest "== 9 passed";
    // mocha "3 passing".
    RE.get_or_init(|| {
        Regex::new(r"(?m)^\s*(?:\#\s+fail\s+0\b|Tests:?\s+\d+\s+passed\b|test result: ok\b|ok\s+\S+\s+(?:[\d.]+s|\(cached\))|=+\s.*\b\d+\s+passed\b|\d+\s+passing\b)").expect("regex")
    })
}

/// Outcome of a failed test command (`PostToolUseFailure`). When a line of
/// several commands failed, but its output shows the tests' own passing
/// summary and no failing one, another command failed: the tests' result is
/// unknown, not a failure (everyday batch 2: `unittest …; mypy --version`
/// with mypy missing).
pub fn test_failure_outcome(payload: &Value, cmd: &str) -> Option<Outcome> {
    let outcome = failure_outcome(payload)?;
    let text = payload["error"].as_str().unwrap_or("");
    // unittest's "OK" counts only next to its "Ran N tests" line: any
    // program can print OK.
    static UNITTEST_OK: OnceLock<Regex> = OnceLock::new();
    let unittest_ok = UNITTEST_OK.get_or_init(|| Regex::new(r"(?m)^Ran\s+\d+\s+tests?\s+in\s+[\d.]+s\s*$[\s\S]*^OK(?:\s+\(.*\))?\s*$").expect("regex"));
    let passed = summary_passed_re().is_match(text) || unittest_ok.is_match(text);
    if outcome == Outcome::Failed && is_compound(cmd) && passed && !summary_failed_re().is_match(text) && !text.contains("FAILED") {
        return None;
    }
    Some(outcome)
}

/// Other common runners, run directly (everyday batch 1: agents ran `node
/// --test` themselves). Only at a command's start, so `pip install pytest`
/// or `grep mocha` isn't a test run.
fn other_runner_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?mx)
            (?:^|[;&|(\n]|\bnpx|\bbunx|\bexec|\buv\s+run|\bpoetry\s+run)[^\S\n]*
            (?:
                node(?:\s+--?[\w-]+(?:=\S+)?)*\s+--test(?:\s|$)
              | deno\s+test\b
              | (?:python[\d.]*(?:\s+-(?:[WX]\s+\S+|[A-Za-ln-z]\w*))*\s+-m\s+)?pytest\b
              | python[\d.]*(?:\s+-(?:[WX]\s+\S+|[A-Za-ln-z]\w*))*\s+-m\s+unittest\b
              | go\s+test\b
              | (?:mocha|ava|uvu)(?:\s|$)
              | (?:bundle\s+exec\s+)?rspec\b
              | mvn\s+(?:-\S+\s+)*test\b
              | (?:\./)?gradlew?\s+test\b
              | dotnet\s+test\b
              | (?:\S*/)?phpunit\b
              | make\s+test\b
            )",
        )
        .expect("regex")
    })
}

fn summary_failed_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Vitest: "Tests  1 failed | 2 passed (3)"; Jest: "Tests:  1 failed, 2 passed, 3 total";
    // cargo: "test result: FAILED. 3 passed; 1 failed"
    // node --test (TAP): "# fail 1"; pytest: "== 2 failed, 7 passed in";
    // go: "--- FAIL:" / "FAIL"; mocha: "1 failing"; unittest: "FAILED (failures=1)";
    // minitest: "N runs, N assertions, 1 failures"; rspec: "3 examples, 1 failure".
    RE.get_or_init(|| {
        Regex::new(r"(?im)^\s*(?:Tests:?\s+(?:.*\b)?[1-9]\d*\s+failed\b|test result: FAILED\b|\#\s+fail\s+[1-9]|=+\s.*\b[1-9]\d*\s+(?:failed|errors?)\b|(?-i:(?:---\s+)?FAIL)\b|(?-i:FAILED)\s+\((?:failures|errors)=\d+|\d+\s+runs?,\s+\d+\s+assertions?,\s+(?:[1-9]\d*\s+failures?|\d+\s+failures?,\s+[1-9]\d*\s+errors?)\b|\d+\s+examples?,\s+[1-9]\d*\s+failures?\b|[1-9]\d*\s+failing\b)").expect("regex")
    })
}

/// Result of a Bash call that fired `PostToolUse` (i.e. it succeeded):
/// an explicit exit code if one is recorded (other agents), else passed,
/// unless a test run's own summary says tests failed (e.g. output piped
/// through `tail`, which hides the test runner's exit code).
pub fn command_outcome(response: &Value, is_test: bool) -> Option<Outcome> {
    if response["interrupted"] == true {
        return None;
    }
    let code = response["exit_code"].as_i64().or_else(|| response["exitCode"].as_i64());
    if let Some(code) = code {
        return Some(if code == 0 { Outcome::Passed } else { Outcome::Failed });
    }
    if is_test {
        let text = format!(
            "{}\n{}",
            response["stdout"].as_str().unwrap_or(""),
            response["stderr"].as_str().unwrap_or("")
        );
        if summary_failed_re().is_match(&text) {
            return Some(Outcome::Failed);
        }
    }
    Some(Outcome::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recognises_test_commands() {
        for cmd in [
            "npx vitest run",
            "vitest",
            "npm test",
            "npm run test",
            "npm run test:unit",
            "pnpm test",
            "yarn jest --watch=false",
            "cd app && npm test",
            "npm t",
            "bun test",
            "cargo test",
            "cargo test -p checks 2>&1 | tail -20",
            "cargo +nightly test",
            "cargo nextest run",
            // Everyday batch 1: the agent ran node's runner directly.
            "node --test",
            "node --test 2>&1 | tail -15",
            "cat cli.js && node --test 2>&1 | tail -15 && node -e 'x'",
            "node --experimental-strip-types --test test/",
            "node --test-reporter=spec --test",
            "deno test",
            "pytest -q",
            "python -m pytest tests/",
            "python3 -m unittest discover",
            "go test ./...",
            "npx mocha",
            "mocha --require test/env test/",
            "bundle exec rspec",
            "rspec spec/",
            "mvn -q test",
            "./gradlew test",
            "dotnet test",
            "vendor/bin/phpunit",
            "make test",
            // Everyday batch 2 (P02): the runner on the line after a heredoc.
            "python3 - <<'EOF'\nopen('a','w').write('x')\nEOF\npython3 -m unittest discover -s tests 2>&1 | tail -15",
            // Everyday batch 4 (P42): interpreter flags before -m.
            "python3 -W error::ResourceWarning -m unittest discover -s tests -q",
            "python -B -m pytest -q",
            "python3 -X dev -m unittest",
        ] {
            assert!(is_test_command(cmd), "{cmd}");
        }
        for cmd in [
            "git commit -m 'add test'",
            "cat src/streak.test.ts",
            "npm install",
            "echo test",
            "npm run build",
            "ls tests/",
            "cargo build --release",
            "cargo clippy --all-targets",
            "node server.js",
            "node -e \"require('./test')\"",
            "go build ./...",
            "pip install pytest",
            "grep -rn mocha package.json",
            "make build",
            "git log --grep test",
            // Text written by a heredoc isn't run.
            "cat > README.md <<'EOF'\n## Testing\ngo test ./...\nEOF",
        ] {
            assert!(!is_test_command(cmd), "{cmd}");
        }
    }

    #[test]
    fn successful_commands_passed_unless_the_summary_says_otherwise() {
        // Real Claude Code shape: no exit code; PostToolUse means it succeeded.
        let real = json!({"stdout": " Tests  3 passed (3)", "stderr": "", "interrupted": false, "isImage": false});
        assert_eq!(command_outcome(&real, true), Some(Outcome::Passed));
        assert_eq!(command_outcome(&json!({"stdout": "building..."}), false), Some(Outcome::Passed));
        assert_eq!(command_outcome(&json!({"stdout": "x", "interrupted": true}), true), None);
        // Explicit exit codes (other agents) win.
        assert_eq!(command_outcome(&json!({"exit_code": 0}), true), Some(Outcome::Passed));
        assert_eq!(command_outcome(&json!({"exitCode": 2}), false), Some(Outcome::Failed));
        // Piped through tail: exit 0, but the summary shows failures.
        let piped = json!({"stdout": " Test Files  1 failed (1)\n      Tests  1 failed | 2 passed (3)\n"});
        let jest = json!({"stderr": "Tests:       1 failed, 4 passed, 5 total"});
        assert_eq!(command_outcome(&piped, true), Some(Outcome::Failed));
        assert_eq!(command_outcome(&jest, true), Some(Outcome::Failed));
        let cargo = json!({"stdout": "test result: FAILED. 3 passed; 1 failed; 0 ignored\n"});
        assert_eq!(command_outcome(&cargo, true), Some(Outcome::Failed));
        assert_eq!(command_outcome(&json!({"stdout": "test result: ok. 4 passed; 0 failed"}), true), Some(Outcome::Passed));
        assert_eq!(command_outcome(&piped, false), Some(Outcome::Passed), "summary only read for test runs");
        // Other runners' summaries, when piped.
        for failed in ["# tests 9\n# pass 8\n# fail 1\n", "===== 2 failed, 7 passed in 0.31s =====", "--- FAIL: TestAdd (0.00s)\nFAIL\n", "  3 passing (12ms)\n  1 failing\n"] {
            assert_eq!(command_outcome(&json!({"stdout": failed}), true), Some(Outcome::Failed), "{failed}");
        }
        for failed in ["12 runs, 17 assertions, 1 failures, 0 errors, 0 skips", "12 runs, 17 assertions, 0 failures, 2 errors, 0 skips", "3 examples, 1 failure"] {
            assert_eq!(command_outcome(&json!({"stdout": failed}), true), Some(Outcome::Failed), "{failed}");
        }
        for passed in ["12 runs, 17 assertions, 0 failures, 0 errors, 0 skips", "3 examples, 0 failures", "fail-fast is off\n# pass 3\n", "# tests 9\n# pass 9\n# fail 0\n", "===== 9 passed in 0.31s =====", "ok  \texample.com/x\t0.01s\n", "  3 passing (12ms)\n"] {
            assert_eq!(command_outcome(&json!({"stdout": passed}), true), Some(Outcome::Passed), "{passed}");
        }
        assert_eq!(
            command_outcome(&json!({"stdout": "Tests: 0 failed, 3 passed, 3 total"}), true),
            Some(Outcome::Passed)
        );
    }

    #[test]
    fn an_unusual_command_with_runner_output_is_a_test_run() {
        let ok = json!({"seq": 1, "event": "PostToolUse", "payload": {"tool_name": "Bash", "tool_input": {"command": "./check.sh"}, "tool_response": {"stdout": "Ran 12 tests in 0.2s\n\nOK\n", "stderr": ""}}});
        let failed = json!({"seq": 2, "event": "PostToolUse", "payload": {"tool_name": "Bash", "tool_input": {"command": "./check.sh | tail -3"}, "tool_response": {"stdout": "Ran 12 tests in 0.2s\n\nFAILED (failures=1)\n", "stderr": ""}}});
        let other = json!({"seq": 3, "event": "PostToolUse", "payload": {"tool_name": "Bash", "tool_input": {"command": "./check.sh"}, "tool_response": {"stdout": "built\n", "stderr": ""}}});
        let n = normalise(&[ok, failed, other], &[]);
        let t: Vec<_> = n.iter().map(|e| e.tool.clone().unwrap()).collect();
        assert!(t[0].is_test_run() && t[0].outcome == Some(Outcome::Passed));
        assert!(t[1].is_test_run() && t[1].outcome == Some(Outcome::Failed), "piped: the summary decides");
        assert!(!t[2].is_test_run());
        // Showing an old run's output isn't a run (code review).
        for cmd in ["cat .last-test-output.txt", "git show HEAD:ci.log | grep Tests", "tail -5 test.log"] {
            let old = json!({"seq": 4, "event": "PostToolUse", "payload": {"tool_name": "Bash", "tool_input": {"command": cmd}, "tool_response": {"stdout": "Tests:  12 passed, 12 total\n", "stderr": ""}}});
            assert!(!normalise(&[old], &[])[0].tool.clone().unwrap().is_test_run(), "{cmd}");
        }
    }

    #[test]
    fn runner_summaries_mark_test_runs() {
        for out in [
            "......\n----------------------------------------------------------------------\nRan 26 tests in 0.112s\n\nOK\n",
            "TAP version 13\n# tests 9\n# suites 0\n# pass 9\n# fail 0\n",
            " Test Files  2 passed (2)\n      Tests  12 passed (12)\n",
            "Tests:       5 passed, 5 total\n",
            "test result: ok. 30 passed; 0 failed; 0 ignored\n",
            "============ 9 passed in 0.31s ============\n",
            "ok  \twc2/count\t0.012s\n",
            "ok  \twc2/count\t(cached)\n",
            "  12 passing (40ms)\n",
            // Everyday batch 6 (R02, R04): Ruby's minitest and RSpec.
            "Finished in 0.003910s, 3069.4 runs/s.\n\n12 runs, 17 assertions, 0 failures, 0 errors, 0 skips\n",
            "Finished in 0.02 seconds (files took 0.1 seconds to load)\n3 examples, 0 failures\n",
        ] {
            assert!(shows_runner_summary(out), "{out}");
        }
        for out in [
            "src/a.test.ts:12:Ran 3 tests in 1s",
            "grep: Tests: no such file",
            "all good\n",
            "OK\n",
            "Ran the script\n",
            "test result is printed below\n",
        ] {
            assert!(!shows_runner_summary(out), "{out}");
        }
    }

    #[test]
    fn a_compound_command_failing_after_passing_tests_is_unknown() {
        // Everyday batch 2 (P05): the tests passed; `mypy` wasn't installed.
        let mypy = json!({"error": "Exit code 1\n.........\n----------------------------------------------------------------------\nRan 26 tests in 0.1s\n\nOK\n/usr/local/bin/python: No module named mypy"});
        let cmd = "ls tests; python -m unittest discover -s tests 2>&1 | tail -5; python -m mypy --version";
        assert_eq!(test_failure_outcome(&mypy, cmd), None);
        // The tests themselves failed: still a failure.
        let failed = json!({"error": "Exit code 1\nRan 26 tests in 0.1s\n\nFAILED (failures=1)\n"});
        assert_eq!(test_failure_outcome(&failed, cmd), Some(Outcome::Failed));
        // A single command that fails is a failure, whatever it printed.
        assert_eq!(test_failure_outcome(&mypy, "python -m unittest discover -s tests"), Some(Outcome::Failed));
        // Code review: a bare OK from another program doesn't excuse a test
        // failure; go's cached result is a passing summary.
        let ok_only = json!({"error": "Exit code 1\nError: vitest crashed\nOK"});
        assert_eq!(test_failure_outcome(&ok_only, "npm test; node scripts/check.js"), Some(Outcome::Failed));
        let cached = json!({"error": "Exit code 1\nok  \twc2/count\t(cached)\nvet: main.go:3: unreachable code"});
        assert_eq!(test_failure_outcome(&cached, "go test ./... && go vet ./..."), None);
        for passed in ["# pass 9\n# fail 0", "Tests  3 passed (3)", "Tests:       5 passed, 5 total", "test result: ok. 4 passed; 0 failed", "ok  \tex.com/x\t0.01s", "===== 9 passed in 0.31s =====", "  3 passing (12ms)"] {
            let e = json!({"error": format!("Exit code 1\n{passed}\nlint: 2 problems")});
            assert_eq!(test_failure_outcome(&e, "npm test && npm run lint"), None, "{passed}");
        }
    }

    #[test]
    fn failure_events_carry_the_exit_code() {
        let failed = json!({"error": "Exit code 1\n FAIL src/a.test.ts\n Tests  1 failed (1)", "is_interrupt": false});
        assert_eq!(failure_outcome(&failed), Some(Outcome::Failed));
        assert_eq!(failure_outcome(&json!({"error": "Exit code 0"})), Some(Outcome::Passed));
        assert_eq!(failure_outcome(&json!({"error": "Command timed out after 2m"})), None);
        assert_eq!(failure_outcome(&json!({"error": "Exit code 1", "is_interrupt": true})), None);
    }

    #[test]
    fn failed_calls_are_marked_on_both_events() {
        let events = vec![
            json!({"seq": 1, "event": "PreToolUse", "payload": {"tool_name": "Edit", "tool_use_id": "t1", "tool_input": {"file_path": "<PROJECT>/a.ts"}}}),
            json!({"seq": 2, "event": "PostToolUseFailure", "payload": {"tool_name": "Edit", "tool_use_id": "t1", "tool_input": {"file_path": "<PROJECT>/a.ts"}, "error": "File has not been read yet"}}),
            json!({"seq": 3, "event": "PreToolUse", "payload": {"tool_name": "Bash", "tool_use_id": "t2", "tool_input": {"command": "npx vitest run"}}}),
            json!({"seq": 4, "event": "PostToolUseFailure", "payload": {"tool_name": "Bash", "tool_use_id": "t2", "tool_input": {"command": "npx vitest run"}, "error": "Exit code 1\nFAIL"}}),
            json!({"seq": 5, "event": "Stop", "payload": {"last_assistant_message": "Done."}}),
        ];
        let n = normalise(&events, &[]);
        assert!(n[0].tool.as_ref().unwrap().failed && !n[0].tool.as_ref().unwrap().is_edit());
        assert!(n[1].tool.as_ref().unwrap().post && !n[1].tool.as_ref().unwrap().is_edit());
        let run = n[3].tool.as_ref().unwrap();
        assert!(run.post && run.is_test_run());
        assert_eq!(run.outcome, Some(Outcome::Failed));
        assert_eq!(n[4].message.as_deref(), Some("Done."));
    }

    #[test]
    fn additive_edits_are_recognised() {
        let ev = |old: &str, new: &str| normalise(&[json!({"seq": 1, "event": "PostToolUse", "payload": {"tool_name": "Edit", "tool_input": {"file_path": "<PROJECT>/a.test.ts", "old_string": old, "new_string": new}}})], &[]);
        assert!(ev("  });\n", "  });\n  it('new', () => {});\n")[0].tool.as_ref().unwrap().additive);
        assert!(!ev("expect(x).toBe(6)", "expect(x).toBe(8)")[0].tool.as_ref().unwrap().additive);
    }

    #[test]
    fn transcript_errors_mark_calls_failed() {
        let events = vec![json!({"seq": 1, "event": "PreToolUse", "payload": {"tool_name": "Edit", "tool_use_id": "t9", "tool_input": {"file_path": "<PROJECT>/a.ts"}}})];
        let transcript = vec![json!({"message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t9", "is_error": true}]}})];
        let n = normalise(&events, &transcript);
        assert!(n[0].tool.as_ref().unwrap().failed);
        assert_eq!(transcript_results(&transcript).get("t9"), Some(&true));
    }

    #[test]
    fn normalises_tool_events() {
        let events = vec![
            json!({"seq": 1, "event": "PreToolUse", "payload": {"tool_name": "Edit", "tool_input": {"file_path": "<PROJECT>/src/a.ts"}}}),
            json!({"seq": 2, "event": "PostToolUse", "payload": {"tool_name": "Bash", "tool_input": {"command": "npx vitest run"}, "tool_response": {"exit_code": 1}}}),
            json!({"seq": 3, "event": "PostToolUse", "payload": {"tool_name": "Write", "tool_input": {"file_path": "<HOME>/.bashrc"}}}),
            json!({"seq": 4, "event": "Stop", "snapshot_error": "git failed"}),
        ];
        let n = normalise(&events, &[]);
        let t0 = n[0].tool.as_ref().unwrap();
        assert!(!t0.post && t0.is_edit());
        assert_eq!(t0.path.as_deref(), Some("src/a.ts"));
        let t1 = n[1].tool.as_ref().unwrap();
        assert!(t1.is_test_run());
        assert_eq!(t1.outcome, Some(Outcome::Failed));
        assert_eq!(n[2].tool.as_ref().unwrap().path, None, "outside the project");
        assert!(n[3].tool.is_none());
        assert_eq!(n[3].capture_problem.as_deref(), Some("snapshot_error: git failed"));
    }

    #[test]
    fn project_paths() {
        assert_eq!(project_path("<PROJECT>/src/a.ts").as_deref(), Some("src/a.ts"));
        assert_eq!(project_path("<PROJECT>"), None);
        assert_eq!(project_path("/etc/hosts"), None);
    }
}
