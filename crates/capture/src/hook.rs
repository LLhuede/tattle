//! `tattle hook`: records one Claude Code hook event.
//!
//! Rules:
//! - Never block or slow the agent: the caller always exits 0 and prints
//!   nothing (stdout from some hooks is shown to the model).
//! - Never lose an event silently: even unparseable input is logged, with a
//!   sequence number, so gaps are visible later.
//! - Never write a secret: every string is cleaned before it touches disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::shadow::Shadow;
use crate::store::{Session, ensure_checker, sanitize_session, write_private};
use crate::text::{Cleaner, Placeholders, Stats};

pub const CV_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Events that get a code snapshot: session start (baseline), each prompt
/// (before the agent works) and each stop (after it finishes).
pub const SNAPSHOT_EVENTS: &[&str] = &["SessionStart", "UserPromptSubmit", "Stop"];

/// Events after which the transcript is copied.
pub const TRANSCRIPT_EVENTS: &[&str] = &["Stop", "SubagentStop", "SessionEnd"];

/// Process environment the hook depends on, separated out for testing.
#[derive(Debug, Clone)]
pub struct HookEnv {
    /// `CLAUDE_PROJECT_DIR`, set by Claude Code for hooks.
    pub project_dir: Option<PathBuf>,
    pub cwd: PathBuf,
    pub home: Option<PathBuf>,
    pub user: Option<String>,
    /// `TATTLE_MANIFEST=1`: ask the agent for a manifest of what it did.
    pub manifest_mode: bool,
    /// `TATTLE_PLUGIN=1`, set by the plugin launcher (`plugin/scripts/tattle`): the
    /// everyday mode. Checks at the end of each turn, tidies old sessions,
    /// and stays out of folders it shouldn't record.
    pub plugin_mode: bool,
    /// The hook came from the installed plugin rather than from a project's
    /// own settings (`TATTLE_SOURCE=project`, as in this repo's
    /// `.claude/settings.json`). A plugin hook stands aside when the project
    /// records itself, so nothing is recorded twice.
    pub from_plugin: bool,
}

fn user_name() -> Option<String> {
    let keys: &[&str] = if cfg!(windows) { &["USERNAME", "USER"] } else { &["USER"] };
    keys.iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

impl HookEnv {
    pub fn from_process() -> Self {
        Self {
            project_dir: std::env::var_os("CLAUDE_PROJECT_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            home: crate::fsx::home_dir(),
            user: user_name(),
            manifest_mode: crate::settings::var("MANIFEST").is_some_and(|v| v == "1"),
            plugin_mode: crate::settings::var("PLUGIN").is_some_and(|v| v == "1"),
            from_plugin: crate::settings::var("SOURCE").is_none_or(|v| v != "project"),
        }
    }

    /// Best guess at the project folder before the payload is parsed.
    pub fn fallback_project(&self) -> PathBuf {
        self.project_dir.clone().unwrap_or_else(|| self.cwd.clone())
    }
}

/// In plugin mode, folders that are never recorded: the home folder or the
/// filesystem root (Claude Code opened outside a project), a project that
/// records itself through its own hooks (research sandboxes, this repo), and
/// on macOS a computer without the developer tools, where running `git`
/// would pop up an install dialog.
pub fn plugin_skip(input: &[u8], env: &HookEnv) -> bool {
    if !env.plugin_mode {
        return false;
    }
    let payload: Value = serde_json::from_slice(input).unwrap_or(Value::Null);
    let project = env.project_dir.clone().or_else(|| payload["cwd"].as_str().map(PathBuf::from)).unwrap_or_else(|| env.cwd.clone());
    let Ok(project) = project.canonicalize() else { return true };
    let is_home = env.home.iter().cloned().chain(crate::fsx::home_dirs()).any(|h| h.canonicalize().is_ok_and(|h| h == project));
    if project.parent().is_none() || is_home {
        return true;
    }
    if env.from_plugin && crate::sandbox::has_recorder_hooks(&project) {
        return true;
    }
    !git_available()
}

#[cfg(target_os = "macos")]
fn git_available() -> bool {
    Path::new("/Library/Developer/CommandLineTools/usr/bin/git").exists()
        || std::process::Command::new("/usr/bin/xcode-select")
            .arg("-p")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

#[cfg(not(target_os = "macos"))]
fn git_available() -> bool {
    true
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Record one hook event. Returns its sequence number.
pub fn record(event_arg: Option<&str>, input: &[u8], env: &HookEnv) -> Result<u64> {
    let started = Instant::now();
    let (payload, parse_error) = match serde_json::from_slice::<Value>(input) {
        Ok(v) => (v, None),
        Err(e) => (Value::Null, Some(e.to_string())),
    };
    let field = |k: &str| payload.get(k).and_then(Value::as_str);

    let event = field("hook_event_name")
        .or(event_arg)
        .unwrap_or("Unknown")
        .to_string();
    let session_id = sanitize_session(field("session_id").unwrap_or("unknown"));
    let project_raw = env
        .project_dir
        .clone()
        .or_else(|| field("cwd").map(PathBuf::from))
        .unwrap_or_else(|| env.cwd.clone());
    let project = crate::fsx::canonical(&project_raw)
        .with_context(|| format!("project folder not found: {}", project_raw.display()))?;
    let transcript_src = field("transcript_path").map(PathBuf::from);
    let subagent_src = field("agent_transcript_path").map(PathBuf::from);
    let agent_id = field("agent_id").map(sanitize_session);

    let checker = ensure_checker(&project)?;
    let session = Session::open(&checker, &session_id)?;
    let cleaner = Cleaner::new(Placeholders::new(
        &[&project_raw, &project],
        env.home.as_deref(),
        env.user.as_deref(),
    ));

    let _lock = session.lock()?;
    let seq = session.next_seq()?;
    let mut stats = Stats::default();
    let mut rec = json!({
        "seq": seq,
        "ts_ms": now_ms(),
        "event": event,
        "session": session_id,
        "cv_version": CV_VERSION,
    });

    match parse_error {
        Some(err) => {
            rec["payload_error"] = json!(err);
            rec["payload_bytes"] = json!(input.len());
        }
        None => rec["payload"] = cleaner.value(payload.clone(), &mut stats),
    }

    if SNAPSHOT_EVENTS.contains(&event.as_str()) {
        let shadow = Shadow::new(&project, &checker);
        match shadow.snapshot(&session_id, seq, &event) {
            Ok(info) => {
                rec["snapshot"] = json!({
                    "commit": info.commit,
                    "ref": info.ref_name,
                    "skipped_large": info.skipped_large,
                });
            }
            Err(e) => rec["snapshot_error"] = json!(cleaner.string(&format!("{e:#}"), &mut stats)),
        }
    }

    if TRANSCRIPT_EVENTS.contains(&event.as_str())
        && let Some(src) = transcript_src
    {
        match copy_transcript(&src, &session.transcript_path(), &cleaner, &mut stats) {
            Ok(lines) => rec["transcript_lines"] = json!(lines),
            Err(e) => {
                rec["transcript_error"] = json!(cleaner.string(&format!("{e:#}"), &mut stats))
            }
        }
    }

    // Manifest (Phase 6b): on a human prompt in manifest mode, set the old
    // one aside so a fresh one is needed; on Stop, keep a cleaned copy.
    let manifest = checker.join(MANIFEST_FILE);
    let is_human_prompt = event == "UserPromptSubmit"
        && !payload["prompt"].as_str().is_some_and(|p| p.trim_start().starts_with("<task-notification>"));
    if is_human_prompt && env.manifest_mode {
        rec["manifest_requested"] = json!(true);
        if manifest.exists() {
            let _ = fs::rename(&manifest, checker.join("manifest.previous.json"));
        }
    }
    if event == "Stop" && manifest.is_file() {
        let dir = session.dir.join("manifests");
        let saved = fs::create_dir_all(&dir).map_err(anyhow::Error::from).and_then(|_| {
            let raw = fs::read_to_string(&manifest)?;
            let cleaned = match serde_json::from_str::<Value>(&raw) {
                Ok(v) => cleaner.value(v, &mut stats),
                Err(e) => json!({ "_cv_unparsed": cleaner.string(&raw, &mut stats), "error": e.to_string() }),
            };
            write_private(&dir.join(format!("{seq:06}.json")), format!("{}\n", serde_json::to_string_pretty(&cleaned)?).as_bytes(), false)
        });
        match saved {
            Ok(()) => rec["manifest_saved"] = json!(true),
            Err(e) => rec["manifest_error"] = json!(cleaner.string(&format!("{e:#}"), &mut stats)),
        }
    }

    if event == "SubagentStop"
        && let Some(src) = subagent_src
    {
        let dir = session.dir.join("subagents");
        let name = agent_id.unwrap_or_else(|| format!("seq{seq}"));
        let copied = std::fs::create_dir_all(&dir)
            .map_err(anyhow::Error::from)
            .and_then(|_| copy_transcript(&src, &dir.join(format!("{name}.jsonl")), &cleaner, &mut stats));
        match copied {
            Ok(lines) => rec["subagent_transcript_lines"] = json!(lines),
            Err(e) => rec["transcript_error"] = json!(cleaner.string(&format!("{e:#}"), &mut stats)),
        }
    }

    rec["redactions"] = json!(stats.redactions);
    rec["trimmed"] = json!(stats.trimmed);
    rec["cv_ms"] = json!(started.elapsed().as_millis() as u64);
    session.append_event(&rec)?;
    Ok(seq)
}

/// Where the agent writes its manifest (inside the self-ignoring folder).
pub const MANIFEST_FILE: &str = "manifest.json";

/// What the agent is asked to write, added to its context on each prompt.
pub const MANIFEST_REQUEST: &str = r#"[tattle recorder] When you have finished this request, write a JSON file at .checker/manifest.json (the folder is ignored by git). A checker uses it to verify your work against the code. Format:
{"schema":"cv-manifest/1","requirements":[{"request":"one requirement, in the user's words","status":"done | partial | not_done","files":["paths you changed for it"],"symbols":["functions, components or types you added or changed for it"],"routes":["METHOD /path you added or changed"],"tests":["names of tests that cover it"],"note":"optional"}],"dependencies_added":[{"name":"package","why":"reason"}],"tests":{"ran":true,"passed":true,"command":"npm test"},"not_done":["anything you skipped or couldn't do"]}
List every requirement in the request, including constraints such as "don't add packages". Only list files, symbols, routes and tests that really exist. Be honest: partial and not_done are fine."#;

/// In manifest mode, what the hook prints back to Claude Code: the request
/// on each human prompt, and (once) a reminder at Stop if no manifest was
/// written. `None` means print nothing.
pub fn manifest_response(input: &[u8], env: &HookEnv) -> Option<String> {
    if !env.manifest_mode {
        return None;
    }
    let payload: Value = serde_json::from_slice(input).ok()?;
    let event = payload["hook_event_name"].as_str()?;
    match event {
        "UserPromptSubmit" if !payload["prompt"].as_str().is_some_and(|p| p.trim_start().starts_with("<task-notification>")) => {
            Some(MANIFEST_REQUEST.to_string())
        }
        "Stop" if payload["stop_hook_active"] != true => {
            let project = env.project_dir.clone().or_else(|| payload["cwd"].as_str().map(PathBuf::from))?;
            if project.join(crate::store::CHECKER_DIR).join(MANIFEST_FILE).is_file() {
                return None;
            }
            let reason = "Before finishing, please write .checker/manifest.json as described earlier (cv-manifest/1), so your work can be checked.";
            Some(json!({ "decision": "block", "reason": reason }).to_string())
        }
        _ => None,
    }
}

/// Copy the agent transcript, cleaning every line. Replaces the previous
/// copy (the transcript only grows). Returns the number of lines.
fn copy_transcript(src: &Path, dest: &Path, cleaner: &Cleaner, stats: &mut Stats) -> Result<usize> {
    let raw = fs::read_to_string(src)
        .with_context(|| format!("could not read transcript {}", src.display()))?;
    let mut out = String::with_capacity(raw.len() / 2);
    let mut lines = 0;
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let cleaned = match serde_json::from_str::<Value>(line) {
            Ok(v) => cleaner.value(v, stats),
            Err(_) => json!({ "_cv_unparsed": cleaner.string(line, stats) }),
        };
        out.push_str(&serde_json::to_string(&cleaned)?);
        out.push('\n');
        lines += 1;
    }
    let tmp = dest.with_extension("jsonl.tmp");
    write_private(&tmp, out.as_bytes(), false)?;
    fs::rename(&tmp, dest)?;
    Ok(lines)
}

/// Log a hook failure to `<project>/.checker/errors.log`. Best effort.
pub fn log_error(env: &HookEnv, message: &str) {
    let project = env.fallback_project();
    if let Ok(checker) = ensure_checker(&project) {
        crate::housekeeping::rotate_log(&checker);
        let cleaner = Cleaner::new(Placeholders::new(
            &[&project],
            env.home.as_deref(),
            env.user.as_deref(),
        ));
        let line = json!({
            "ts_ms": now_ms(),
            "cv_version": CV_VERSION,
            "error": cleaner.string(message, &mut Stats::default()),
        });
        let _ = write_private(
            &checker.join("errors.log"),
            format!("{line}\n").as_bytes(),
            true,
        );
    }
}
