//! End-to-end tests for the everyday (plugin) mode: drive the real `tattle`
//! binary the way the plugin's hooks do (`TATTLE_PLUGIN=1`), then check what
//! the person would see.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const CV: &str = env!("CARGO_BIN_EXE_tattle");
const SESSION: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

struct Fixture {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    project: PathBuf,
    transcript: PathBuf,
    tool_id: std::cell::Cell<u32>,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let project = home.join("Code").join("shop");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("package.json"), r#"{"name":"shop","scripts":{"test":"vitest run"}}"#).unwrap();
        fs::write(project.join("src/cart.ts"), "export const total = (xs: number[]) => xs.reduce((a, b) => a + b, 0);\n").unwrap();
        let transcript = home.join(".claude/projects/shop/session.jsonl");
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        fs::write(&transcript, "").unwrap();
        Self { _tmp: tmp, home, project, transcript, tool_id: std::cell::Cell::new(0) }
    }

    fn run_hook(&self, project: &Path, payload: &Value, extra_env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(CV);
        cmd.arg("hook")
            .env("CLAUDE_PROJECT_DIR", project)
            .env("HOME", &self.home)
            .env("USER", "tester")
            .env("TATTLE_PLUGIN", "1")
            .env_remove("TATTLE_SOURCE")
            .env_remove("TATTLE_REPORT_COMMAND");
        for (k, v) in extra_env {
            cmd.env(k, v);
        }
        let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(payload.to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "hook must always exit 0");
        out
    }

    /// Send one hook event; returns what the hook printed.
    fn hook(&self, event: &str, extra: Value) -> String {
        let mut payload = json!({
            "session_id": SESSION,
            "transcript_path": self.transcript,
            "cwd": self.project,
            "hook_event_name": event,
        });
        for (k, v) in extra.as_object().unwrap() {
            payload[k] = v.clone();
        }
        String::from_utf8(self.run_hook(&self.project, &payload, &[]).stdout).unwrap()
    }

    fn prompt(&self, text: &str) {
        assert_eq!(self.hook("UserPromptSubmit", json!({"prompt": text})), "");
    }

    /// A Bash call; `fails` fires PostToolUseFailure like Claude Code does.
    fn bash(&self, command: &str, fails: bool) {
        let n = self.tool_id.get() + 1;
        self.tool_id.set(n);
        let id = format!("toolu_{n:04}");
        let input = json!({"tool_name": "Bash", "tool_input": {"command": command}, "tool_use_id": id});
        self.hook("PreToolUse", input.clone());
        if fails {
            let mut p = input;
            p["error"] = json!("Exit code 1\n FAIL  src/cart.test.ts\n Tests  1 failed | 2 passed (3)");
            self.hook("PostToolUseFailure", p);
        } else {
            let mut p = input;
            p["tool_response"] = json!({"stdout": " Tests  3 passed (3)", "stderr": "", "interrupted": false, "isImage": false});
            self.hook("PostToolUse", p);
        }
    }

    fn stop(&self, message: &str) -> String {
        let line = json!({"type": "assistant", "message": {"role": "assistant", "content": [{"type": "text", "text": message}]}});
        let mut t = fs::read_to_string(&self.transcript).unwrap();
        t.push_str(&format!("{line}\n"));
        fs::write(&self.transcript, t).unwrap();
        self.hook("Stop", json!({"stop_hook_active": false, "last_assistant_message": message}))
    }

    fn report(&self) -> String {
        let out = Command::new(CV).args(["report", "--project"]).arg(&self.project).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }
}

fn system_message(printed: &str) -> String {
    let v: Value = serde_json::from_str(printed).unwrap_or_else(|e| panic!("not JSON ({e}): {printed:?}"));
    assert_eq!(v.as_object().unwrap().len(), 1, "only a message for the person, never a block: {v}");
    v["systemMessage"].as_str().unwrap().to_string()
}

#[test]
fn a_false_tests_pass_claim_is_shown_once_in_plain_words() {
    let f = Fixture::new();
    assert_eq!(f.hook("SessionStart", json!({"source": "startup"})), "");
    f.prompt("fix the cart total for empty carts");
    f.bash("npm test", true);
    let printed = f.stop("Done! All tests pass.");
    let msg = system_message(&printed);
    assert_eq!(msg, "⚠️ tattle: Claude said the tests pass, but they failed the last time it ran them. Type /tattle:report for details.");
    assert!(!msg.contains("claims."), "no internal ids: {msg}");

    // Next turn: same problem still there, but it isn't repeated.
    f.prompt("thanks, also add a comment");
    assert_eq!(f.stop("Added the comment."), "");

    let report = f.report();
    assert!(report.starts_with("tattle checked this session (2 requests)."), "{report}");
    // It stays in the report, tied to the request it was about.
    assert!(
        report.contains("Worth a look:\n- Request 1 (\"fix the cart total for empty carts\"): Claude said the tests pass, but they failed the last time it ran them."),
        "{report}"
    );
}

#[test]
fn report_items_say_which_request() {
    let f = Fixture::new();
    f.hook("SessionStart", json!({"source": "startup"}));
    f.prompt("add a comment to the cart total");
    assert_eq!(f.stop("Added the comment."), "");
    f.prompt("now make empty carts return 0 and tell me the tests pass");
    let msg = system_message(&f.stop("Done. All tests pass."));
    assert_eq!(msg, "⚠️ tattle: Claude said the tests pass, but it never ran them. Type /tattle:report for details.");
    let report = f.report();
    assert!(
        report.contains("- Request 2 (\"now make empty carts return 0 and tell me the tests pass\"): Claude said the tests pass, but it never ran them. (`claims.tests_pass_unverified`)"),
        "{report}"
    );
}

#[test]
fn a_clean_turn_prints_nothing() {
    let f = Fixture::new();
    f.hook("SessionStart", json!({"source": "startup"}));
    f.prompt("make the tests pass");
    f.bash("npm test", true);
    f.bash("npm test", false);
    assert_eq!(f.stop("Fixed the off-by-one; all tests pass now."), "");
    assert!(f.report().contains("found nothing to report"));
}

#[test]
fn the_report_command_can_be_renamed() {
    let f = Fixture::new();
    f.hook("SessionStart", json!({}));
    f.prompt("add a discount");
    let mut payload = json!({"session_id": SESSION, "transcript_path": f.transcript, "cwd": f.project, "hook_event_name": "Stop",
        "last_assistant_message": "All tests pass."});
    payload["stop_hook_active"] = json!(false);
    let out = f.run_hook(&f.project, &payload, &[("TATTLE_REPORT_COMMAND", "/tattle-report")]);
    let msg = system_message(&String::from_utf8(out.stdout).unwrap());
    assert!(msg.contains("it never ran them") && msg.ends_with("Type /tattle-report for details."), "{msg}");
}

#[test]
fn live_check_matches_the_exported_recording() {
    let f = Fixture::new();
    f.hook("SessionStart", json!({}));
    f.prompt("fix the cart total for empty carts");
    f.bash("npm test", true);
    fs::write(f.project.join("src/cart.ts"), "export const total = (xs: number[]) => xs.reduce((a, b) => a + b, 0) || 0;\n").unwrap();
    f.stop("Done! All tests pass.");
    let out = f._tmp.path().join("export");
    let status = Command::new(CV)
        .args(["record", "export", "--prompt-id", "X", "--run", "1", "--project"])
        .arg(&f.project)
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(status.status.success(), "{}", String::from_utf8_lossy(&status.stderr));
    let exported = checks::run(&checks::Recording::load(&out).unwrap());
    let live = checks::run(&checks::Recording::load_live(&f.project.canonicalize().unwrap(), SESSION).unwrap());
    assert!(!live.findings.is_empty());
    assert_eq!(live.canonical_json(), exported.canonical_json());
}

#[test]
fn the_home_folder_is_never_recorded() {
    let f = Fixture::new();
    let payload = json!({"session_id": SESSION, "cwd": f.home, "hook_event_name": "SessionStart"});
    f.run_hook(&f.home, &payload, &[]);
    assert!(!f.home.join(".checker").exists());
}

#[test]
fn a_project_that_records_itself_is_not_recorded_twice() {
    let f = Fixture::new();
    fs::create_dir_all(f.project.join(".claude")).unwrap();
    let settings = json!({"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "TATTLE_SOURCE=project \"$CLAUDE_PROJECT_DIR\"/plugin/scripts/tattle hook"}]}]}});
    fs::write(f.project.join(".claude/settings.json"), settings.to_string()).unwrap();
    let payload = json!({"session_id": SESSION, "cwd": f.project, "hook_event_name": "SessionStart"});
    f.run_hook(&f.project, &payload, &[]);
    assert!(!f.project.join(".checker").exists(), "the plugin's own hook stands aside");
    f.run_hook(&f.project, &payload, &[("TATTLE_SOURCE", "project")]);
    assert!(f.project.join(".checker/sessions").join(SESSION).join("events.jsonl").is_file(), "the project's hook records");
}

#[test]
fn old_sessions_are_tidied_at_session_start() {
    let f = Fixture::new();
    let sessions = f.project.join(".checker/sessions");
    for i in 0..35 {
        let dir = sessions.join(format!("old-{i:02}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("events.jsonl"), "{}\n").unwrap();
    }
    f.hook("SessionStart", json!({}));
    let left = fs::read_dir(&sessions).unwrap().count();
    assert_eq!(left, capture::housekeeping::KEEP_SESSIONS + 1, "30 most recent + the current one");
    assert!(sessions.join(SESSION).is_dir());
}

#[test]
fn report_before_anything_is_recorded() {
    let f = Fixture::new();
    assert!(f.report().contains("hasn't recorded anything"));
}

/// The plugin's hooks and this repo's development hooks cover exactly the
/// events the recorder expects, each calling the launcher.
#[test]
fn hook_files_match_the_recorder_events() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (file, needle) in [
        ("plugin/hooks/hooks.json", "\"${CLAUDE_PLUGIN_ROOT}/scripts/tattle\" hook"),
        (".claude/settings.json", "TATTLE_SOURCE=project TATTLE_REPORT_COMMAND=/tattle-report \"$CLAUDE_PROJECT_DIR/plugin/scripts/tattle\" hook"),
    ] {
        let v: Value = serde_json::from_str(&fs::read_to_string(root.join(file)).unwrap()).unwrap();
        let hooks = v["hooks"].as_object().unwrap();
        let names: Vec<&str> = hooks.keys().map(String::as_str).collect();
        let mut expected: Vec<&str> = capture::sandbox::HOOK_EVENTS.iter().map(|(e, _)| *e).collect();
        let mut got = names.clone();
        expected.sort();
        got.sort();
        assert_eq!(got, expected, "{file}");
        for (event, is_tool) in capture::sandbox::HOOK_EVENTS {
            let entry = &hooks[*event][0];
            assert_eq!(entry["hooks"][0]["command"], *needle, "{file} {event}");
            assert_eq!(entry["matcher"].as_str() == Some("*"), *is_tool, "{file} {event}");
            let timeout = entry["hooks"][0]["timeout"].as_u64().unwrap();
            assert!(timeout >= 30 && (*event != "Stop" || timeout as u128 > live_deadline_secs()), "{file} {event}");
        }
    }
}

fn live_deadline_secs() -> u128 {
    20 // tattle's end-of-turn check deadline (crates/tattle/src/live.rs, DEADLINE)
}

fn config(f: &Fixture, args: &[&str]) -> String {
    let out = Command::new(CV).arg("config").args(args).arg("--project").arg(&f.project).env_remove("TATTLE_MUTE").env_remove("TATTLE_TELL_CLAUDE").env_remove("TATTLE_REPORT_COMMAND").output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn a_muted_rule_is_never_shown_but_the_report_says_so() {
    let f = Fixture::new();
    f.hook("SessionStart", json!({"source": "startup"}));
    assert!(config(&f, &["mute", "claims.tests_pass_false"]).starts_with("Muted claims.tests_pass_false"));
    f.prompt("fix the cart total for empty carts");
    f.bash("npm test", true);
    assert_eq!(f.stop("Done! All tests pass."), "", "muted: nothing at the end of the turn");
    let report = f.report();
    assert!(!report.contains("they failed the last time"), "{report}");
    assert!(report.contains("Muted here, and left out above: claims.tests_pass_false."), "{report}");
    // Unmuted: it's new to the person, so it's shown.
    config(&f, &["unmute", "claims.tests_pass_false"]);
    f.prompt("and again");
    let msg = system_message(&f.stop("Done! All tests pass."));
    assert!(msg.contains("they failed the last time it ran them"), "{msg}");
    assert!(f.report().contains("(`claims.tests_pass_false`)"), "the report names the rule to mute");
}

#[test]
fn with_tell_claude_on_a_new_alert_goes_back_to_claude_once() {
    let f = Fixture::new();
    f.hook("SessionStart", json!({"source": "startup"}));
    assert!(config(&f, &["tell-claude", "on"]).contains("also sends new alerts"));
    f.prompt("fix the cart total for empty carts");
    f.bash("npm test", true);
    let printed = f.stop("Done! All tests pass.");
    let v: Value = serde_json::from_str(&printed).unwrap();
    assert_eq!(v["decision"], "block", "{v}");
    assert!(v["reason"].as_str().unwrap().contains("1. Claude said the tests pass, but they failed the last time it ran them."), "{v}");
    assert!(v["systemMessage"].as_str().unwrap().starts_with("⚠️ tattle:"), "the person still sees it: {v}");
    // Claude answers; Stop fires again with stop_hook_active: never a second block.
    f.bash("npm test", true);
    let again = f.hook("Stop", json!({"stop_hook_active": true, "last_assistant_message": "You're right, one test still fails."}));
    assert!(again.is_empty() || !again.contains("\"decision\""), "{again}");
    // Off again: back to a message for the person only.
    config(&f, &["tell-claude", "off"]);
    f.prompt("next");
    f.bash("npm test", false);
    assert_eq!(f.stop("Done, and I didn't change the tests."), "");
}

#[test]
fn the_plugins_own_commands_are_not_checked() {
    let f = Fixture::new();
    f.hook("SessionStart", json!({"source": "startup"}));
    config(&f, &["tell-claude", "on"]);
    // The rule name reads like a code name that doesn't exist; it isn't work.
    f.prompt("/tattle:mute claims.tests_pass_unverified");
    assert_eq!(f.stop("Muted claims.tests_pass_unverified in this project."), "", "nothing to check, nothing sent");
    // A mistyped rule is a plain message, not an error.
    let out = Command::new(CV).args(["config", "mute", "not.a_rule", "--project"]).arg(&f.project).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("tattle has no rule called 'not.a_rule'"));
}
