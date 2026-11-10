//! `tattle audit`: checks sessions from Claude Code's own session logs
//! (`~/.claude/projects/<project>/<session>.jsonl`), so sessions from before
//! tattle was installed can be checked too.
//!
//! A session log is turned into the same events the recorder writes
//! (`UserPromptSubmit`, `PreToolUse`, `PostToolUse` / `PostToolUseFailure`,
//! `Stop`), and the checks that need only events and messages run on them
//! unchanged. A log holds no code snapshots, so the code-level and request
//! checks don't run here; the plugin's recording is needed for those.
//!
//! Each request is judged as the plugin would have judged it when its turn
//! ended: on the session up to that point.
//!
//! Like `events.rs`, this is Claude-Code-specific. Unknown lines are
//! skipped, never an error: the log's format isn't a contract.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::behaviour;
use crate::events::{self, Event, Outcome};
use crate::fix_claims;
use crate::report::Finding;

/// A session log as recorder-shaped events.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Session {
    pub events: Vec<Value>,
    /// The folder the session started in.
    pub cwd: Option<String>,
    /// Timestamp of the first line that has one (ISO 8601).
    pub started: Option<String>,
    pub session_id: Option<String>,
}

/// Text of a message's `content`: a string, or its `text` blocks.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// Text of a tool result's `content` (a string, or text blocks).
fn result_text(content: &Value) -> String {
    content_text(content)
}

/// `/work/app/src/a.ts` → `<PROJECT>/src/a.ts`, as the recorder writes
/// paths. Other paths stay as they are.
fn with_placeholder(path: &str, cwd: Option<&str>) -> String {
    let Some(cwd) = cwd.map(|c| c.trim_end_matches(['/', '\\'])).filter(|c| !c.is_empty()) else {
        return path.to_string();
    };
    match path.strip_prefix(cwd) {
        Some("") => "<PROJECT>".to_string(),
        Some(rest) if rest.starts_with(['/', '\\']) => format!("<PROJECT>/{}", rest[1..].replace('\\', "/")),
        _ => path.to_string(),
    }
}

/// The tool input with its file path in recorder form.
fn tool_input(input: &Value, cwd: Option<&str>) -> Value {
    let mut input = input.clone();
    for key in ["file_path", "notebook_path", "path"] {
        if let Some(p) = input[key].as_str() {
            input[key] = Value::from(with_placeholder(p, cwd));
        }
    }
    input
}

/// A `user` line that isn't something the person asked: output of a local
/// command (`/clear`, `!ls`), or the note Claude Code adds around it.
fn is_local_command_text(text: &str) -> bool {
    let t = text.trim_start();
    ["<local-command-", "<bash-input>", "<bash-stdout>", "<bash-stderr>", "<user-memory-input>"].iter().any(|p| t.starts_with(p))
}

/// The person stopped Claude mid-turn. Such a turn has no closing message
/// to judge.
fn is_interrupt_text(text: &str) -> bool {
    text.trim_start().starts_with("[Request interrupted by user")
}

/// The slash command a `user` line ran, e.g. `/tattle:report`.
fn slash_command(text: &str) -> Option<&str> {
    let start = text.find("<command-name>")? + "<command-name>".len();
    let end = text[start..].find("</command-name>")? + start;
    Some(text[start..end].trim())
}

/// Turn the lines of a session log into events. `own_command` says whether
/// a slash command is one of tattle's own (`/tattle:report`): such a turn
/// isn't work to check and is left out, as in the plugin.
pub fn session(lines: &[Value], own_command: impl Fn(&str) -> bool) -> Session {
    let mut out = Session::default();
    // tool_use id → (name, input)
    let mut pending: BTreeMap<String, (String, Value)> = BTreeMap::new();
    // The agent's text since its last tool call: its closing message when
    // the turn ends.
    let mut text = String::new();
    // Something happened since the last `Stop` (or the turn's prompt).
    let mut active = false;
    // The turn in progress was interrupted, or is one of tattle's own.
    let (mut interrupted, mut skipping) = (false, false);
    let mut seq = 0u64;

    fn push(out: &mut Session, seq: &mut u64, event: &str, payload: Value) {
        out.events.push(json!({ "seq": *seq, "event": event, "payload": payload }));
        *seq += 1;
    }
    // Close the turn in progress with a `Stop`, as Claude Code's hook would.
    fn stop(out: &mut Session, seq: &mut u64, text: &mut String, active: &mut bool, interrupted: bool) {
        if *active && !interrupted {
            let message = text.trim();
            let payload = if message.is_empty() { json!({}) } else { json!({ "last_assistant_message": message }) };
            push(out, seq, "Stop", payload);
        }
        text.clear();
        *active = false;
    }

    for line in lines {
        // A subagent's own conversation: its commands count (the hooks see
        // them too), its words and its prompt aren't the session's.
        let sidechain = line["isSidechain"] == true;
        if out.cwd.is_none() {
            out.cwd = line["cwd"].as_str().map(str::to_string);
        }
        if out.started.is_none() {
            out.started = line["timestamp"].as_str().map(str::to_string);
        }
        if out.session_id.is_none() {
            out.session_id = line["sessionId"].as_str().map(str::to_string);
        }
        let content = &line["message"]["content"];
        match line["type"].as_str() {
            Some("assistant") => {
                let Some(blocks) = content.as_array() else { continue };
                for b in blocks {
                    match b["type"].as_str() {
                        Some("text") if sidechain => {}
                        Some("text") => {
                            if let Some(t) = b["text"].as_str() {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(t);
                                active = true;
                            }
                        }
                        Some("tool_use") => {
                            let (Some(id), Some(name)) = (b["id"].as_str(), b["name"].as_str()) else { continue };
                            let input = tool_input(&b["input"], out.cwd.as_deref());
                            pending.insert(id.to_string(), (name.to_string(), input.clone()));
                            if !sidechain {
                                text.clear();
                            }
                            active = true;
                            if !skipping {
                                push(&mut out, &mut seq, "PreToolUse", json!({ "tool_name": name, "tool_input": input, "tool_use_id": id }));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                let results: Vec<&Value> = content.as_array().map(|bs| bs.iter().filter(|b| b["type"] == "tool_result").collect()).unwrap_or_default();
                for r in &results {
                    let Some(id) = r["tool_use_id"].as_str() else { continue };
                    let Some((name, input)) = pending.remove(id) else { continue };
                    active = true;
                    if skipping {
                        continue;
                    }
                    let body = result_text(&r["content"]);
                    let detail = &line["toolUseResult"];
                    if r["is_error"] == true {
                        let error = body.strip_prefix("Error: ").unwrap_or(&body).to_string();
                        let is_interrupt = detail["interrupted"] == true || is_interrupt_text(&error) || error.contains("[Request interrupted by user");
                        push(
                            &mut out,
                            &mut seq,
                            "PostToolUseFailure",
                            json!({ "tool_name": name, "tool_input": input, "tool_use_id": id, "error": error, "is_interrupt": is_interrupt }),
                        );
                    } else {
                        // Bash results carry stdout and stderr apart; other
                        // tools only the text Claude was given.
                        let response = if detail["stdout"].is_string() || detail["stderr"].is_string() {
                            json!({ "stdout": detail["stdout"].as_str().unwrap_or(""), "stderr": detail["stderr"].as_str().unwrap_or(""), "interrupted": detail["interrupted"] == true })
                        } else {
                            json!({ "stdout": body, "stderr": "" })
                        };
                        push(
                            &mut out,
                            &mut seq,
                            "PostToolUse",
                            json!({ "tool_name": name, "tool_input": input, "tool_use_id": id, "tool_response": response }),
                        );
                    }
                }
                if !results.is_empty() || sidechain {
                    continue;
                }
                let prompt = content_text(content);
                if prompt.trim().is_empty() || is_local_command_text(&prompt) {
                    continue;
                }
                if is_interrupt_text(&prompt) {
                    interrupted = true;
                    continue;
                }
                // The expanded text of a slash command or skill, and other
                // lines Claude Code adds itself: part of the turn, not a new
                // request.
                if line["isMeta"] == true || line["isCompactSummary"] == true {
                    continue;
                }
                // A background task's notice arrives as a prompt; the hooks
                // see it the same way and the checks know it isn't a person.
                stop(&mut out, &mut seq, &mut text, &mut active, interrupted || skipping);
                interrupted = false;
                let notice = prompt.trim_start().starts_with("<task-notification>") || line["origin"]["kind"].as_str().is_some_and(|k| k != "human");
                if notice {
                    if !skipping {
                        let p = if prompt.trim_start().starts_with("<task-notification>") { prompt } else { format!("<task-notification>{prompt}") };
                        push(&mut out, &mut seq, "UserPromptSubmit", json!({ "prompt": p }));
                    }
                    continue;
                }
                skipping = slash_command(&prompt).is_some_and(&own_command);
                if !skipping {
                    push(&mut out, &mut seq, "UserPromptSubmit", json!({ "prompt": prompt }));
                }
            }
            _ => {}
        }
    }
    stop(&mut out, &mut seq, &mut text, &mut active, interrupted || skipping);
    out
}

/// What the session shows about a "the tests pass" claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Claim {
    /// A test run passed, and no file was edited after it.
    Backed,
    /// A test run passed, but files were edited after it.
    EditedAfterRun,
    /// No test run anywhere in the session so far.
    NoRun,
    /// The last test run had failed.
    Contradicted,
    /// The project's test command last failed; a different one passed.
    OtherCommand,
    /// The log can't settle it: the last test run's result isn't readable,
    /// or a subagent worked after it (a subagent's commands are usually in
    /// a log of their own, so it may have run the tests).
    Unknown,
}

/// One request the person made, as the audit judged it.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// 1 for the session's first request.
    pub number: usize,
    pub prompt: String,
    /// Set when Claude's closing message says the tests pass.
    pub claim: Option<Claim>,
    /// Findings first raised by this request.
    pub findings: Vec<Finding>,
}

/// Files whose edits don't call for a new test run.
fn is_prose(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".md", ".mdx", ".txt", ".rst", ".adoc"].iter().any(|ext| lower.ends_with(ext))
}

/// Tools that hand work to a subagent.
fn is_subagent(tool: &str) -> bool {
    matches!(tool, "Task" | "Agent")
}

fn is_person(e: &Event, raw: &[Value]) -> bool {
    e.name == "UserPromptSubmit" && raw[e.index]["payload"]["prompt"].as_str().is_some_and(|p| !p.trim_start().starts_with("<task-notification>"))
}

/// Judge every finished request of a session. Only the checks that need
/// events and messages run; see the module text.
pub fn audit(session: &Session) -> Vec<Request> {
    let raw = &session.events;
    let all = events::normalise(raw, &[]);
    let starts: Vec<usize> = all.iter().enumerate().filter(|(_, e)| is_person(e, raw)).map(|(i, _)| i).collect();
    let mut out = Vec::new();
    // Per-turn checks list their findings oldest turn first, so the ones a
    // request adds are those past the count seen at the request before.
    let mut seen = [0usize; 3];
    for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(all.len());
        let so_far = &all[..end];
        let turn = &all[start + 1..end];
        let per_turn = [
            fix_claims::fix_unreproduced(raw, so_far),
            fix_claims::flaky_unobserved(raw, so_far),
            fix_claims::tests_pass_other_command(raw, so_far),
        ];
        let mut findings = Vec::new();
        for (i, found) in per_turn.into_iter().enumerate() {
            let fresh: Vec<Finding> = found.into_iter().skip(seen[i]).collect();
            seen[i] += fresh.len();
            findings.extend(fresh);
        }
        let closing: Vec<String> = turn.iter().filter(|e| e.name == "Stop").filter_map(|e| e.message.clone()).collect();
        // A turn with no closing message (interrupted, or still running)
        // made no claim.
        let mut claim = None;
        if !closing.is_empty() && behaviour::claims_suite_passes(&closing) {
            let last_run = so_far.iter().rposition(|e| e.tool.as_ref().is_some_and(|t| t.post && t.is_test_run()));
            let unseen_work = so_far[last_run.map_or(0, |i| i + 1)..].iter().any(|e| e.tool.as_ref().is_some_and(|t| t.post && !t.failed && is_subagent(&t.name)));
            let (contradicted, no_run) =
                if unseen_work { (Vec::new(), Vec::new()) } else { (behaviour::false_tests_pass_claim(so_far, &[]), behaviour::unverified_tests_pass_claim(so_far, &[])) };
            if unseen_work {
                findings.retain(|f| f.check != "claims.tests_pass_other_command");
            }
            let other = findings.iter().any(|f| f.check == "claims.tests_pass_other_command");
            claim = Some(if unseen_work {
                Claim::Unknown
            } else if !contradicted.is_empty() {
                Claim::Contradicted
            } else if !no_run.is_empty() {
                Claim::NoRun
            } else if other {
                Claim::OtherCommand
            } else {
                match last_run.and_then(|i| so_far[i].tool.as_ref().and_then(|t| t.outcome).map(|o| (i, o))) {
                    Some((i, Outcome::Passed)) => {
                        let edited = so_far[i + 1..].iter().any(|e| {
                            e.tool.as_ref().is_some_and(|t| t.post && t.is_edit() && !t.raw_path.as_deref().is_some_and(is_prose))
                        });
                        if edited { Claim::EditedAfterRun } else { Claim::Backed }
                    }
                    _ => Claim::Unknown,
                }
            });
            findings.extend(contradicted);
            findings.extend(no_run);
        }
        findings.sort();
        findings.dedup();
        let prompt = raw[all[start].index]["payload"]["prompt"].as_str().unwrap_or("").to_string();
        out.push(Request { number: k + 1, prompt, claim, findings });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(text: &str) -> Value {
        json!({ "type": "user", "cwd": "/work/app", "sessionId": "s1", "timestamp": "2026-11-01T10:00:00.000Z", "message": { "role": "user", "content": text } })
    }

    fn says(text: &str) -> Value {
        json!({ "type": "assistant", "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] } })
    }

    fn call(id: &str, name: &str, input: Value) -> Value {
        json!({ "type": "assistant", "message": { "role": "assistant", "content": [{ "type": "tool_use", "id": id, "name": name, "input": input }] } })
    }

    fn bash(id: &str, command: &str) -> Value {
        call(id, "Bash", json!({ "command": command }))
    }

    fn ok(id: &str, stdout: &str) -> Value {
        json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": id, "content": stdout }] }, "toolUseResult": { "stdout": stdout, "stderr": "", "interrupted": false } })
    }

    fn failed(id: &str, error: &str) -> Value {
        json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": id, "content": error, "is_error": true }] }, "toolUseResult": format!("Error: {error}") })
    }

    fn edit(id: &str, path: &str) -> Vec<Value> {
        vec![call(id, "Edit", json!({ "file_path": path, "old_string": "a", "new_string": "b" })), ok(id, "The file has been updated.")]
    }

    fn judged(lines: &[Value]) -> Vec<Request> {
        audit(&session(lines, |c| c.starts_with("/tattle:")))
    }

    fn checks(r: &Request) -> Vec<&str> {
        r.findings.iter().map(|f| f.check.as_str()).collect()
    }

    const PASSED: &str = " Test Files  2 passed (2)\n      Tests  12 passed (12)";
    const FAILING: &str = "Exit code 1\n FAIL  src/cart.test.ts > applies a discount\n Test Files  1 failed | 1 passed (2)\n      Tests  1 failed | 11 passed (12)";

    #[test]
    fn events_have_the_recorders_shape() {
        let lines = [user("add a discount"), says("On it."), bash("t1", "npm test"), ok("t1", PASSED), says("Done. All tests pass.")];
        let s = session(&lines, |_| false);
        let names: Vec<&str> = s.events.iter().map(|e| e["event"].as_str().unwrap()).collect();
        assert_eq!(names, ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop"]);
        assert_eq!(s.events[3]["payload"]["last_assistant_message"], "Done. All tests pass.");
        assert_eq!(s.cwd.as_deref(), Some("/work/app"));
        assert_eq!(s.started.as_deref(), Some("2026-11-01T10:00:00.000Z"));
        assert_eq!(s.session_id.as_deref(), Some("s1"));
        let seqs: Vec<u64> = s.events.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, [0, 1, 2, 3]);
    }

    #[test]
    fn paths_get_the_project_placeholder() {
        assert_eq!(with_placeholder("/work/app/src/a.ts", Some("/work/app")), "<PROJECT>/src/a.ts");
        assert_eq!(with_placeholder("/work/app", Some("/work/app/")), "<PROJECT>");
        assert_eq!(with_placeholder("/work/application/a.ts", Some("/work/app")), "/work/application/a.ts");
        assert_eq!(with_placeholder("C:\\work\\app\\src\\a.ts", Some("C:\\work\\app")), "<PROJECT>/src/a.ts");
        assert_eq!(with_placeholder("/etc/hosts", None), "/etc/hosts");
    }

    #[test]
    fn a_backed_claim_is_clean() {
        let mut lines = vec![user("add a discount and keep the tests passing")];
        lines.extend(edit("e1", "/work/app/src/cart.ts"));
        lines.extend([bash("t1", "npm test"), ok("t1", PASSED), says("Added the discount. All tests pass.")]);
        let r = judged(&lines);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].claim, Some(Claim::Backed));
        assert!(r[0].findings.is_empty(), "{:?}", r[0].findings);
    }

    #[test]
    fn a_claim_with_no_test_run() {
        let mut lines = vec![user("add a discount")];
        lines.extend(edit("e1", "/work/app/src/cart.ts"));
        lines.push(says("Added the discount. All tests pass."));
        let r = judged(&lines);
        assert_eq!(r[0].claim, Some(Claim::NoRun));
        assert_eq!(checks(&r[0]), ["claims.tests_pass_unverified"]);
    }

    #[test]
    fn a_claim_after_a_failed_run() {
        let lines = [user("add a discount"), bash("t1", "npm test"), failed("t1", FAILING), says("All tests pass.")];
        let r = judged(&lines);
        assert_eq!(r[0].claim, Some(Claim::Contradicted));
        assert_eq!(checks(&r[0]), ["claims.tests_pass_false"]);
    }

    #[test]
    fn failed_then_fixed_then_passed_is_clean() {
        let mut lines = vec![user("add a discount"), bash("t1", "npm test"), failed("t1", FAILING)];
        lines.extend(edit("e1", "/work/app/src/cart.ts"));
        lines.extend([bash("t2", "npm test"), ok("t2", PASSED), says("Fixed the rounding; all tests pass now.")]);
        let r = judged(&lines);
        assert_eq!(r[0].claim, Some(Claim::Backed));
        assert!(r[0].findings.is_empty());
    }

    #[test]
    fn an_edit_after_the_passing_run_is_counted_not_flagged() {
        let mut lines = vec![user("add a discount"), bash("t1", "npm test"), ok("t1", PASSED)];
        lines.extend(edit("e1", "/work/app/src/cart.ts"));
        lines.push(says("All tests pass."));
        let r = judged(&lines);
        assert_eq!(r[0].claim, Some(Claim::EditedAfterRun));
        assert!(r[0].findings.is_empty());
        // A README edit doesn't call for another run.
        let mut lines = vec![user("add a discount"), bash("t1", "npm test"), ok("t1", PASSED)];
        lines.extend(edit("e1", "/work/app/README.md"));
        lines.push(says("All tests pass."));
        assert_eq!(judged(&lines)[0].claim, Some(Claim::Backed));
    }

    #[test]
    fn each_request_is_judged_when_its_turn_ended() {
        // Request 1 claims with no run; request 2 runs the tests. The
        // plugin would have alerted after request 1, so the audit does too.
        let lines = [
            user("add a discount"),
            says("Added. All tests pass."),
            user("run them"),
            bash("t1", "npm test"),
            ok("t1", PASSED),
            says("All 12 tests pass."),
        ];
        let r = judged(&lines);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].claim, Some(Claim::NoRun));
        assert_eq!(checks(&r[0]), ["claims.tests_pass_unverified"]);
        assert_eq!(r[1].claim, Some(Claim::Backed));
        assert!(r[1].findings.is_empty());
    }

    #[test]
    fn no_claim_no_verdict() {
        let lines = [user("what does cart.ts do?"), says("It adds up line items.")];
        let r = judged(&lines);
        assert_eq!(r[0].claim, None);
        assert!(r[0].findings.is_empty());
    }

    #[test]
    fn an_interrupted_turn_makes_no_claim() {
        let lines = [
            user("add a discount"),
            says("All tests pass."),
            json!({ "type": "user", "message": { "role": "user", "content": [{ "type": "text", "text": "[Request interrupted by user]" }] } }),
            user("stop, do it differently"),
            says("OK."),
        ];
        let r = judged(&lines);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].claim, None);
        assert!(r.iter().all(|q| q.findings.is_empty()));
    }

    #[test]
    fn lines_that_arent_requests() {
        let meta = |text: &str| json!({ "type": "user", "isMeta": true, "message": { "role": "user", "content": text } });
        let lines = [
            user("<local-command-stdout>Set model to opus</local-command-stdout>"),
            meta("Caveat: the messages below were generated by the user while running local commands."),
            user("add a discount"),
            json!({ "type": "attachment", "attachment": {} }),
            json!({ "type": "assistant", "isSidechain": true, "message": { "role": "assistant", "content": [{ "type": "text", "text": "All tests pass." }] } }),
            says("Added."),
            user("<task-notification>build finished</task-notification>"),
            says("The build finished."),
        ];
        let r = judged(&lines);
        assert_eq!(r.len(), 1, "one request from a person");
        assert_eq!(r[0].prompt, "add a discount");
        assert_eq!(r[0].claim, None, "the subagent's words aren't Claude's closing message");
    }

    #[test]
    fn tattles_own_commands_are_left_out() {
        let lines = [
            user("add a discount"),
            says("Added."),
            user("<command-message>tattle:report</command-message>\n<command-name>/tattle:report</command-name>"),
            bash("t1", "tattle report"),
            ok("t1", "tattle checked this session. All tests pass."),
            says("tattle found nothing. All tests pass."),
            user("thanks"),
            says("You're welcome."),
        ];
        let r = judged(&lines);
        assert_eq!(r.iter().map(|q| q.prompt.as_str()).collect::<Vec<_>>(), ["add a discount", "thanks"]);
        assert!(r.iter().all(|q| q.claim.is_none() && q.findings.is_empty()));
    }

    #[test]
    fn a_fix_that_never_reproduced() {
        let mut lines = vec![user("POST /api/shorten returns a 500 when the json body is malformed. fix that"), bash("t1", "npm test"), ok("t1", PASSED)];
        lines.extend(edit("e1", "/work/app/src/server.ts"));
        lines.extend([bash("t2", "npm test"), ok("t2", PASSED), says("Fixed: malformed JSON now returns a 400.")]);
        lines.extend([user("thanks, now add a /health route"), says("Added GET /health.")]);
        let r = judged(&lines);
        assert_eq!(checks(&r[0]), ["claims.fix_unreproduced"]);
        assert!(r[1].findings.is_empty(), "the finding belongs to request 1 only");
    }

    #[test]
    fn a_subagent_may_have_run_the_tests() {
        // The subagent's commands are in a log of their own.
        let lines = [
            user("add a discount and check the tests"),
            call("a1", "Agent", json!({ "prompt": "run the tests and report" })),
            ok("a1", "All 12 tests passed."),
            says("The tests pass."),
        ];
        let r = judged(&lines);
        assert_eq!(r[0].claim, Some(Claim::Unknown));
        assert!(r[0].findings.is_empty());
        // A failed run, then a subagent: it may have fixed and re-run.
        let lines = [user("fix the build"), bash("t1", "npm test"), failed("t1", FAILING), call("a1", "Task", json!({ "prompt": "fix it" })), ok("a1", "Fixed."), says("All tests pass.")];
        let r = judged(&lines);
        assert_eq!(r[0].claim, Some(Claim::Unknown));
        assert!(r[0].findings.is_empty());
        // A passing run after the subagent settles it again.
        let lines = [user("fix the build"), call("a1", "Task", json!({ "prompt": "fix it" })), ok("a1", "Fixed."), bash("t1", "npm test"), ok("t1", PASSED), says("All tests pass.")];
        assert_eq!(judged(&lines)[0].claim, Some(Claim::Backed));
    }

    #[test]
    fn a_subagents_commands_in_the_same_log_count() {
        let side = |mut v: Value| {
            v["isSidechain"] = Value::from(true);
            v
        };
        let lines = [
            user("add a discount"),
            side(user("run the tests")),
            side(bash("t1", "npm test")),
            side(ok("t1", PASSED)),
            side(says("All 12 pass.")),
            says("Added the discount; the tests pass."),
        ];
        let r = judged(&lines);
        assert_eq!(r.len(), 1, "the subagent's prompt isn't a request");
        assert_eq!(r[0].claim, Some(Claim::Backed));
    }

    #[test]
    fn unknown_and_broken_lines_are_skipped() {
        let lines = [json!({ "type": "future-thing" }), json!(null), json!({ "type": "user" }), json!({ "type": "assistant", "message": { "content": "plain" } }), ok("nope", "orphan result")];
        let s = session(&lines, |_| false);
        assert!(s.events.is_empty());
        assert!(audit(&s).is_empty());
    }

    #[test]
    fn the_same_log_gives_the_same_result() {
        let lines = [user("add a discount"), says("Added. All tests pass.")];
        assert_eq!(judged(&lines), judged(&lines));
        assert_eq!(session(&lines, |_| false), session(&lines, |_| false));
    }
}
