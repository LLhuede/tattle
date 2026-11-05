//! End-to-end tests for the recorder: drive the real `tattle` binary with hook
//! payloads shaped like Claude Code's, then inspect what was written.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

const CV: &str = env!("CARGO_BIN_EXE_tattle");
const SESSION: &str = "11111111-2222-3333-4444-555555555555";
// Fake, and split so secret scanners don't take it for a real key.
const FAKE_SECRET: &str = concat!("sk-ant-", "api03-FAKE-test-key-not-real");

struct Fixture {
    _tmp: tempfile::TempDir,
    project: PathBuf,
    home: PathBuf,
    transcript: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let project = home.join("cv-sandbox").join("G1-run1");
        fs::create_dir_all(project.join("src")).unwrap();
        fs::write(project.join("src/app.ts"), "export const x = 1;\n").unwrap();
        let transcript = home.join(".claude/projects/x/session.jsonl");
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        Self {
            _tmp: tmp,
            project,
            home,
            transcript,
        }
    }

    fn checker(&self) -> PathBuf {
        self.project.join(".checker")
    }

    fn session_dir(&self) -> PathBuf {
        self.checker().join("sessions").join(SESSION)
    }

    /// Run `tattle hook` with a raw stdin payload.
    fn hook_raw(&self, stdin: &[u8]) -> Output {
        let mut child = Command::new(CV)
            .arg("hook")
            .env("CLAUDE_PROJECT_DIR", &self.project)
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, &self.home)
            .env(if cfg!(windows) { "USERNAME" } else { "USER" }, "testuser")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin).unwrap();
        child.wait_with_output().unwrap()
    }

    fn hook(&self, event: &str, extra: Value) -> Output {
        let mut payload = json!({
            "session_id": SESSION,
            "transcript_path": self.transcript,
            "cwd": self.project,
            "hook_event_name": event,
        });
        for (k, v) in extra.as_object().unwrap() {
            payload[k] = v.clone();
        }
        let out = self.hook_raw(payload.to_string().as_bytes());
        assert!(out.status.success(), "hook must always exit 0");
        assert!(out.stdout.is_empty(), "hook must print nothing on stdout");
        out
    }

    fn events(&self) -> Vec<Value> {
        fs::read_to_string(self.session_dir().join("events.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("--git-dir")
            .arg(self.checker().join("shadow.git"))
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    /// A short but realistic session: start, prompt, read, edit, test run, stop.
    fn record_session(&self) {
        self.hook("SessionStart", json!({"source": "startup"}));
        self.hook(
            "UserPromptSubmit",
            json!({"prompt": format!("add a streak counter. my key is {FAKE_SECRET}")}),
        );
        let file = self.project.join("src/app.ts");
        self.hook(
            "PreToolUse",
            json!({"tool_name": "Read", "tool_input": {"file_path": file}}),
        );
        self.hook(
            "PostToolUse",
            json!({"tool_name": "Read", "tool_input": {"file_path": file},
                   "tool_response": {"content": "x".repeat(10_000)}}),
        );
        fs::write(&file, "export const x = 1;\nexport const streak = 0;\n").unwrap();
        self.hook(
            "PostToolUse",
            json!({"tool_name": "Edit", "tool_input": {"file_path": file, "old_string": "a", "new_string": "b"}}),
        );
        self.hook(
            "PostToolUse",
            json!({"tool_name": "Bash", "tool_input": {"command": "npm test"},
                   "tool_response": {"stdout": "1 passed", "stderr": "", "exit_code": 0}}),
        );
        let transcript = [
            json!({"type": "user", "version": "2.1.304", "cwd": self.project,
                   "message": {"role": "user", "content": format!("my key is {FAKE_SECRET}")}}),
            json!({"type": "assistant", "version": "2.1.304",
                   "message": {"role": "assistant", "content": "Done! All tests pass."}}),
        ];
        let text: String = transcript.iter().map(|l| format!("{l}\n")).collect();
        fs::write(&self.transcript, text).unwrap();
        self.hook("Stop", json!({"stop_hook_active": false}));
    }
}

fn all_text_under(dir: &Path) -> String {
    let mut out = String::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(t) = fs::read_to_string(&p) {
                out.push_str(&t);
            }
        }
    }
    out
}

#[test]
fn records_every_event_in_order() {
    let f = Fixture::new();
    f.record_session();
    let events = f.events();
    let names: Vec<&str> = events.iter().map(|e| e["event"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUse", "PostToolUse", "Stop"]
    );
    let seqs: Vec<u64> = events.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, (1..=7).collect::<Vec<_>>());
    assert_eq!(events[5]["payload"]["tool_response"]["exit_code"], 0);
}

#[test]
fn secrets_never_reach_disk() {
    let f = Fixture::new();
    f.record_session();
    let everything = all_text_under(&f.checker());
    assert!(!everything.contains(FAKE_SECRET), "secret leaked into .checker");
    assert!(everything.contains("[REDACTED:anthropic-key]"));
    assert_eq!(f.events()[1]["redactions"], 1);
}

#[test]
fn paths_and_username_are_replaced() {
    let f = Fixture::new();
    f.record_session();
    let events = fs::read_to_string(f.session_dir().join("events.jsonl")).unwrap();
    let project = f.project.canonicalize().unwrap();
    assert!(!events.contains(&*project.to_string_lossy()));
    assert!(!events.contains(&*f.project.to_string_lossy()));
    assert!(events.contains("<PROJECT>/src/app.ts"));
    assert!(events.contains("<HOME>/.claude/projects"));
}

#[test]
fn long_tool_output_is_trimmed_with_hash() {
    let f = Fixture::new();
    f.record_session();
    let read = &f.events()[3];
    let content = read["payload"]["tool_response"]["content"].as_str().unwrap();
    assert!(content.len() < 3000);
    assert!(content.contains("cv-trimmed bytes=10000 sha256="));
    assert_eq!(read["trimmed"], 1);
}

#[test]
fn snapshots_at_start_prompt_and_stop() {
    let f = Fixture::new();
    f.record_session();
    let events = f.events();
    let with_snapshot: Vec<&str> = events
        .iter()
        .filter(|e| e.get("snapshot").is_some())
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert_eq!(with_snapshot, ["SessionStart", "UserPromptSubmit", "Stop"]);

    let before = events[1]["snapshot"]["commit"].as_str().unwrap();
    let after = events[6]["snapshot"]["commit"].as_str().unwrap();
    assert_eq!(f.git(&["show", &format!("{before}:src/app.ts")]), "export const x = 1;\n");
    assert!(f.git(&["show", &format!("{after}:src/app.ts")]).contains("streak"));
    // Snapshots form a chain.
    assert_eq!(f.git(&["rev-parse", &format!("{after}^")]).trim(), before);
}

#[test]
fn snapshots_leave_out_dependencies_secrets_and_large_files() {
    let f = Fixture::new();
    let p = &f.project;
    fs::create_dir_all(p.join("node_modules/lib")).unwrap();
    fs::write(p.join("node_modules/lib/index.js"), "x").unwrap();
    fs::write(p.join(".env"), "API_KEY=abc").unwrap();
    fs::write(p.join(".env.example"), "API_KEY=").unwrap();
    fs::write(p.join("data.sqlite"), "db").unwrap();
    fs::write(p.join("big.json"), vec![b'a'; 2 * 1024 * 1024]).unwrap();
    fs::write(p.join(".gitignore"), "ignored.txt\n").unwrap();
    fs::write(p.join("ignored.txt"), "x").unwrap();
    f.hook("SessionStart", json!({}));

    let commit = f.events()[0]["snapshot"]["commit"].as_str().unwrap().to_string();
    let files = f.git(&["ls-tree", "-r", "--name-only", &commit]);
    let files: Vec<&str> = files.lines().collect();
    assert_eq!(files, [".env.example", ".gitignore", "src/app.ts"]);
    assert_eq!(f.events()[0]["snapshot"]["skipped_large"], json!(["big.json"]));
}

#[test]
fn the_users_own_git_repo_is_untouched() {
    let f = Fixture::new();
    let run = |args: &[&str]| {
        let out = Command::new("git").args(args).current_dir(&f.project).output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    run(&["init", "-q"]);
    run(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"]);
    let head_before = run(&["rev-parse", "HEAD"]);
    f.record_session();
    assert_eq!(run(&["rev-parse", "HEAD"]), head_before);
    // .checker/ ignores itself, so it never shows up as untracked.
    assert_eq!(run(&["status", "--porcelain"]).trim(), "?? src/");
}

#[test]
fn transcript_is_copied_cleaned() {
    let f = Fixture::new();
    f.record_session();
    let t = fs::read_to_string(f.session_dir().join("transcript.jsonl")).unwrap();
    assert_eq!(t.lines().count(), 2);
    assert!(!t.contains(FAKE_SECRET));
    assert!(t.contains("<PROJECT>"));
    assert_eq!(f.events()[6]["transcript_lines"], 2);
}

#[test]
fn bad_input_is_recorded_not_dropped() {
    let f = Fixture::new();
    let out = f.hook_raw(b"this is not json");
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    let unknown = f.checker().join("sessions/unknown/events.jsonl");
    let line: Value = serde_json::from_str(fs::read_to_string(unknown).unwrap().trim()).unwrap();
    assert_eq!(line["seq"], 1);
    assert!(line["payload_error"].is_string());
}

#[test]
fn missing_project_folder_still_exits_zero() {
    let out = Command::new(CV)
        .arg("hook")
        .env("CLAUDE_PROJECT_DIR", "/definitely/not/here")
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
}

#[test]
fn hook_with_unexpected_arguments_still_exits_zero() {
    let out = Command::new(CV)
        .args(["hook", "--weird", "flags"])
        .current_dir(std::env::temp_dir())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success());
}

#[test]
fn concurrent_hooks_get_unique_sequence_numbers() {
    let f = Fixture::new();
    std::thread::scope(|s| {
        for i in 0..20 {
            let f = &f;
            s.spawn(move || {
                f.hook(
                    "PostToolUse",
                    json!({"tool_name": "Bash", "tool_input": {"command": format!("echo {i}")}}),
                );
            });
        }
    });
    let mut seqs: Vec<u64> = f.events().iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    seqs.sort();
    assert_eq!(seqs, (1..=20).collect::<Vec<_>>());
}

#[test]
fn tool_events_are_fast() {
    let f = Fixture::new();
    f.hook("SessionStart", json!({}));
    let start = std::time::Instant::now();
    for _ in 0..10 {
        f.hook("PostToolUse", json!({"tool_name": "Read", "tool_input": {"file_path": "x"}}));
    }
    let per_call = start.elapsed() / 10;
    // Generous bound for a debug build on a busy machine; release is far faster.
    assert!(per_call.as_millis() < 250, "PostToolUse took {per_call:?} per call");
    let cv_ms: Vec<u64> = f.events()[1..].iter().map(|e| e["cv_ms"].as_u64().unwrap()).collect();
    assert!(cv_ms.iter().all(|ms| *ms < 50), "internal time per tool event: {cv_ms:?}");
}

// ---------- export ----------

fn export_cmd(f: &Fixture, out: &Path, corpus: &Path, extra: &[&str]) -> Output {
    Command::new(CV)
        .args(["record", "export", "--prompt-id", "G1", "--run", "1"])
        .arg("--project")
        .arg(&f.project)
        .arg("--out")
        .arg(out)
        .arg("--corpus")
        .arg(corpus)
        .args(extra)
        .output()
        .unwrap()
}

#[test]
fn export_writes_minimal_recording_within_budget() {
    let f = Fixture::new();
    f.record_session();
    let corpus = f.home.join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    fs::write(
        corpus.join("G1.json"),
        json!({"id": "G1", "expected_facts": [{"id": "storage", "description": "Browser storage"}]}).to_string(),
    )
    .unwrap();
    let out_dir = f.home.join("fixtures/G1/run1");
    let out = export_cmd(&f, &out_dir, &corpus, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let mut names: Vec<String> = fs::read_dir(&out_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["events.jsonl", "labels.json", "meta.json", "snapshots.bundle", "transcript.jsonl"]
    );

    let meta: Value = serde_json::from_str(&fs::read_to_string(out_dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["agent_version"], "2.1.304");
    // The exported transcript keeps the conversation only.
    let t = fs::read_to_string(out_dir.join("transcript.jsonl")).unwrap();
    assert_eq!(t.lines().count(), 2);
    assert!(t.contains("All tests pass") && !t.contains("\"cwd\""));
    assert_eq!(meta["events"], 7);
    assert_eq!(meta["snapshots"].as_array().unwrap().len(), 3);
    assert_eq!(meta["over_budget"], false);

    let labels: Value = serde_json::from_str(&fs::read_to_string(out_dir.join("labels.json")).unwrap()).unwrap();
    assert_eq!(labels["intent_facts"][0]["fact_id"], "storage");
    assert_eq!(labels["intent_facts"][0]["verdict"], "");

    // The bundle restores the snapshots on its own.
    let heads = Command::new("git")
        .args(["bundle", "list-heads"])
        .arg(out_dir.join("snapshots.bundle"))
        .output()
        .unwrap();
    assert!(heads.status.success());
    assert_eq!(String::from_utf8_lossy(&heads.stdout).lines().count(), 3);

    assert!(!all_text_under(&out_dir).contains(FAKE_SECRET));
}

#[test]
fn export_refuses_secrets_in_snapshots_unless_allowed() {
    let f = Fixture::new();
    fs::write(f.project.join("src/config.ts"), format!("export const KEY = \"{FAKE_SECRET}\";\n")).unwrap();
    f.record_session();
    let corpus = f.home.join("corpus");
    let out_dir = f.home.join("fixtures/G1/run1");

    let refused = export_cmd(&f, &out_dir, &corpus, &[]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("src/config.ts"));
    assert!(!out_dir.exists(), "nothing may be written when refusing");

    let allowed = export_cmd(&f, &out_dir, &corpus, &["--allow-snapshot-secrets"]);
    assert!(allowed.status.success(), "{}", String::from_utf8_lossy(&allowed.stderr));
    let meta: Value = serde_json::from_str(&fs::read_to_string(out_dir.join("meta.json")).unwrap()).unwrap();
    assert_eq!(meta["snapshot_secrets_allowed"][0]["path"], "src/config.ts");
}

#[test]
fn export_accepts_a_relative_output_folder() {
    let f = Fixture::new();
    f.record_session();
    let corpus = f.home.join("corpus");
    let out = Command::new(CV)
        .args(["record", "export", "--prompt-id", "G1", "--run", "1", "--out", "fixtures/G1/run1"])
        .arg("--project")
        .arg(&f.project)
        .arg("--corpus")
        .arg(&corpus)
        .current_dir(&f.home)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(f.home.join("fixtures/G1/run1/snapshots.bundle").is_file());
    assert!(!f.project.join("fixtures").exists(), "nothing written inside the project");
}

#[test]
fn failed_export_leaves_nothing_behind() {
    let f = Fixture::new();
    f.record_session();
    // Break the snapshots so the bundle can't be made.
    capture::fsx::remove_dir_all(&f.checker().join("shadow.git/objects")).unwrap();
    let out_dir = f.home.join("fixtures/G1/run1");
    let out = export_cmd(&f, &out_dir, &f.home.join("corpus"), &[]);
    assert!(!out.status.success());
    assert!(!out_dir.exists());
}

#[test]
fn export_never_overwrites_an_existing_recording() {
    let f = Fixture::new();
    f.record_session();
    let corpus = f.home.join("corpus");
    let out_dir = f.home.join("fixtures/G1/run1");
    assert!(export_cmd(&f, &out_dir, &corpus, &[]).status.success());
    fs::write(out_dir.join("labels.json"), "{\"labelled\": true}").unwrap();
    assert!(!export_cmd(&f, &out_dir, &corpus, &[]).status.success());
    assert_eq!(fs::read_to_string(out_dir.join("labels.json")).unwrap(), "{\"labelled\": true}");
}

#[test]
fn restore_rebuilds_the_final_app_from_a_recording() {
    let f = Fixture::new();
    fs::create_dir_all(f.project.join("node_modules/x")).unwrap();
    fs::write(f.project.join("node_modules/x/i.js"), "x").unwrap();
    f.record_session();
    let corpus = f.home.join("corpus");
    let rec = f.home.join("fixtures/G1/run1");
    assert!(export_cmd(&f, &rec, &corpus, &[]).status.success());

    let base = f.home.join("bases/G1");
    let out = Command::new(CV)
        .args(["record", "restore", "--from"])
        .arg(&rec)
        .arg("--out")
        .arg(&base)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("-Stop"));
    assert_eq!(
        fs::read_to_string(base.join("src/app.ts")).unwrap(),
        "export const x = 1;\nexport const streak = 0;\n"
    );
    assert!(!base.join("node_modules").exists());
    let leftovers: Vec<_> = fs::read_dir(base.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["G1"], "temporary folder must be cleaned up");
}

#[test]
fn exported_transcripts_keep_only_the_conversation() {
    let long = "x".repeat(2000);
    let text = [
        json!({"type": "attachment", "attachment": {"big": long}}),
        json!({"type": "api-request-shape", "shape": {"big": long}}),
        json!({"type": "assistant", "uuid": "u1", "version": "2.1.307", "message": {"role": "assistant", "model": "m", "content": [
            {"type": "thinking", "thinking": "secret reasoning here"},
            {"type": "text", "text": "Running the tests."},
            {"type": "tool_use", "id": "t1", "name": "Write", "input": {"file_path": "<PROJECT>/a.ts", "content": long}},
        ]}}),
        json!({"type": "user", "uuid": "u2", "toolUseResult": {"big": long}, "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "is_error": false, "content": long},
        ]}}),
    ]
    .iter()
    .map(|l| format!("{l}\n"))
    .collect::<String>();
    let slim = capture::export::slim_transcript(&text);
    let lines: Vec<Value> = slim.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["message"]["content"][0], json!({"type": "thinking", "chars": 21}));
    assert_eq!(lines[0]["message"]["content"][1]["text"], "Running the tests.");
    assert_eq!(lines[0]["version"], "2.1.307");
    let content = lines[0]["message"]["content"][2]["input"]["content"].as_str().unwrap();
    assert!(content.chars().count() <= 301);
    assert_eq!(lines[1]["message"]["content"][0], json!({"type": "tool_result", "tool_use_id": "t1", "is_error": false}));
    assert!(slim.len() < 2000, "slimmed to {} bytes", slim.len());
}

// ---------- sandbox ----------

#[test]
fn sandbox_init_sets_up_hooks_and_copies_base() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("base");
    fs::create_dir_all(base.join("src")).unwrap();
    fs::create_dir_all(base.join("node_modules/x")).unwrap();
    fs::create_dir_all(base.join(".checker")).unwrap();
    fs::write(base.join("src/a.ts"), "a").unwrap();
    let dir = tmp.path().join("sandbox/E2-run1");

    let out = Command::new(CV)
        .args(["sandbox", "init"])
        .arg(&dir)
        .arg("--base")
        .arg(&base)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(fs::read_to_string(dir.join("src/a.ts")).unwrap(), "a");
    assert!(!dir.join("node_modules").exists());
    assert!(!dir.join(".checker").exists());
    let settings: Value =
        serde_json::from_str(&fs::read_to_string(dir.join(".claude/settings.local.json")).unwrap()).unwrap();
    let cmd = settings["hooks"]["PostToolUse"][0]["hooks"][0]["command"].as_str().unwrap();
    assert!(cmd.ends_with(" hook"));

    // A second init into the same folder is refused.
    let again = Command::new(CV).args(["sandbox", "init"]).arg(&dir).output().unwrap();
    assert!(!again.status.success());
}
