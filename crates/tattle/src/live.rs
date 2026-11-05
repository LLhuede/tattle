//! The everyday (plugin) mode: check the session while it is still being
//! recorded, and tell the person in plain words.
//!
//! - When Claude finishes a turn (`Stop`), every check runs on the live
//!   session. Alerts that haven't been shown before in this session become
//!   one short line for the person (`systemMessage`). Claude itself is not
//!   told; nothing is printed when there's nothing new.
//! - `tattle report` prints the whole session's findings in plain words (the
//!   plugin's `/tattle:report` command), each with its rule name.
//! - Settings (`config.rs`): muted rules are never shown, and with "tell
//!   Claude" on, new alerts are also handed back to Claude, once per turn.
//! - When a session starts, old sessions are tidied away and the snapshot
//!   store is packed in the background.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use capture::hook::HookEnv;
use capture::store::{CHECKER_DIR, latest_session, sanitize_session, write_private};
use checks::plain::{self, Level};

use crate::config::Config;
use serde_json::{Value, json};

/// What the person types for details (the plugin's skill).
pub const DEFAULT_REPORT_COMMAND: &str = "/tattle:report";

/// The end-of-turn check gives up after this long and says nothing, so a
/// very large project never holds Claude up.
pub const DEADLINE: Duration = Duration::from_secs(20);

/// Alerts already shown in a session (finding key → `{text, request}`), so
/// each is shown once and the report still lists it after later turns.
const SHOWN_FILE: &str = "shown.json";

fn read_shown(session_dir: &Path) -> BTreeMap<String, Value> {
    fs::read_to_string(session_dir.join(SHOWN_FILE)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Shown alerts. Keys are `rule|file|message`; older sessions stored just
/// the sentence.
fn shown_alerts(session_dir: &Path) -> Vec<plain::Shown> {
    read_shown(session_dir)
        .into_iter()
        .map(|(key, v)| {
            let (text, request) = match &v {
                Value::String(s) => (s.clone(), None),
                _ => (v["text"].as_str().unwrap_or("").to_string(), v["request"].as_u64().map(|n| n as usize)),
            };
            let rule = key.split('|').next().unwrap_or("").to_string();
            plain::Shown { key, rule, text, request }
        })
        .collect()
}

/// `/tattle:` for `/tattle:report`, so the other commands read `/tattle:mute`.
pub(crate) fn command_prefix() -> String {
    let report = report_command();
    report.strip_suffix("report").unwrap_or("/tattle:").to_string()
}

fn report_command() -> String {
    capture::settings::var("REPORT_COMMAND").filter(|c| !c.trim().is_empty()).unwrap_or_else(|| DEFAULT_REPORT_COMMAND.to_string())
}

/// Project folder and session id of a hook payload.
fn project_and_session(payload: &Value, env: &HookEnv) -> Option<(PathBuf, String)> {
    let project = env.project_dir.clone().or_else(|| payload["cwd"].as_str().map(PathBuf::from))?;
    let session = sanitize_session(payload["session_id"].as_str().unwrap_or("unknown"));
    Some((capture::fsx::canonical(&project).ok()?, session))
}

/// Plain sentences for the alerts in this session that the person hasn't
/// been shown yet. Marks them as shown.
/// Rules the person muted are skipped (and not marked as shown, so they
/// appear if unmuted).
pub fn new_alerts(project: &Path, session: &str, settings: &Config) -> Result<Vec<String>> {
    let mut rec = checks::Recording::load_live(project, session)?;
    // A turn that only ran one of tattle's own commands isn't work to check.
    let prefix = command_prefix();
    rec.drop_turns(|p| p.starts_with(prefix.as_str()));
    let report = checks::run(&rec);
    let seqs: Vec<u64> = checks::human_prompt_seqs(&rec).into_iter().map(|(s, _)| s).collect();
    let mut shown = read_shown(&rec.dir);
    let mut fresh: Vec<&checks::report::Finding> = Vec::new();
    for f in &report.findings {
        let p = plain::explain(f);
        if p.level != Level::Alert || settings.is_muted(&f.check) {
            continue;
        }
        if let std::collections::btree_map::Entry::Vacant(slot) = shown.entry(plain::alert_key(f)) {
            slot.insert(json!({ "text": p.text, "request": plain::request_of(f, &seqs) }));
            fresh.push(f);
        }
    }
    // Several files of one rule read as one sentence.
    let mut out: Vec<String> = Vec::new();
    for (_, _, p) in plain::sentences(&fresh, &seqs) {
        if !out.contains(&p.text) {
            out.push(p.text);
        }
    }
    if !out.is_empty() {
        write_private(&rec.dir.join(SHOWN_FILE), format!("{}\n", serde_json::to_string_pretty(&shown)?).as_bytes(), false)?;
    }
    Ok(out)
}

/// What the `Stop` hook prints in plugin mode: `{"systemMessage": …}` with
/// new alerts, or `None` (print nothing).
pub fn stop_response(input: &[u8], env: &HookEnv) -> Option<String> {
    let payload: Value = serde_json::from_slice(input).ok()?;
    if payload["hook_event_name"] != "Stop" {
        return None;
    }
    let (project, session) = project_and_session(&payload, env)?;
    let settings = Config::load(&project);
    let (tx, rx) = mpsc::channel();
    let (p, s, c) = (project.clone(), session.clone(), settings.clone());
    std::thread::spawn(move || {
        let result = std::panic::catch_unwind(|| new_alerts(&p, &s, &c)).unwrap_or_else(|_| Err(anyhow::anyhow!("panic in the end-of-turn check")));
        let _ = tx.send(result);
    });
    let received = rx.recv_timeout(DEADLINE);
    // Pack this turn's snapshots for the next check (in the background).
    capture::shadow::Shadow::new(&project, &project.join(CHECKER_DIR)).pack_in_background();
    let alerts = match received {
        Ok(Ok(alerts)) => alerts,
        Ok(Err(e)) => {
            capture::hook::log_error(env, &format!("end-of-turn check: {e:#}"));
            return None;
        }
        Err(_) => {
            capture::hook::log_error(env, &format!("end-of-turn check took over {}s; skipped", DEADLINE.as_secs()));
            return None;
        }
    };
    let line = plain::stop_line(&alerts, &report_command())?;
    // Sending alerts to Claude (opt-in): Claude goes on with the alerts as
    // its next instruction. Only once per turn: when Claude stops again
    // after that, `stop_hook_active` is set and the person just sees any
    // new alert, so it can never loop.
    if settings.tell_claude && payload["stop_hook_active"] != true
        && let Some(reason) = plain::claude_message(&alerts)
    {
        return Some(json!({ "systemMessage": line, "decision": "block", "reason": reason }).to_string());
    }
    Some(json!({ "systemMessage": line }).to_string())
}

/// At session start in plugin mode: delete old sessions. Best effort.
pub fn session_start(input: &[u8], env: &HookEnv) {
    let Ok(payload) = serde_json::from_slice::<Value>(input) else { return };
    if payload["hook_event_name"] != "SessionStart" {
        return;
    }
    let Some((project, session)) = project_and_session(&payload, env) else { return };
    let checker = project.join(CHECKER_DIR);
    if let Err(e) = capture::housekeeping::prune_sessions(&project, &checker, &session, SystemTime::now()) {
        capture::hook::log_error(env, &format!("tidying old sessions: {e:#}"));
    }
    // Pack the starting snapshot so the first end-of-turn check reads it fast.
    capture::shadow::Shadow::new(&project, &checker).pack_in_background();
}

/// `tattle report`: the session's findings in plain words.
pub fn report(project: &Path, session: Option<&str>) -> Result<String> {
    let project = capture::fsx::canonical(project).with_context(|| format!("folder not found: {}", project.display()))?;
    let checker = project.join(CHECKER_DIR);
    let session = match session {
        Some(s) => Some(sanitize_session(s)),
        None => latest_session(&checker)?,
    };
    let Some(session) = session else {
        return Ok("tattle hasn't recorded anything in this folder yet. It starts with your next request to Claude.\n".to_string());
    };
    let mut rec = checks::Recording::load_live(&project, &session)?;
    // The request that asked for this report is still running.
    rec.drop_open_turn();
    // Earlier reports and settings changes aren't work to check.
    let prefix = command_prefix();
    rec.drop_turns(|p| p.starts_with(prefix.as_str()));
    let found = checks::run(&rec);
    // A warning shown after an earlier turn stays in the report even when a
    // later turn's checks no longer raise it.
    let earlier = shown_alerts(&rec.dir);
    let requests = checks::human_prompt_seqs(&rec);
    let config = Config::load(&project);
    let settings = plain::ReportSettings { muted: config.mute, tell_claude: config.tell_claude, command_prefix: prefix };
    Ok(plain::report(&found.findings, &earlier, &requests, &settings))
}
