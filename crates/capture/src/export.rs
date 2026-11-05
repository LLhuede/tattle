//! `tattle record export`: copy one recorded session into the test corpus,
//! keeping only what the checker needs (see docs/DECISIONS.md, "Recording
//! storage").
//!
//! Output folder:
//! ```text
//! events.jsonl       cleaned hook events (already redacted at capture)
//! transcript.jsonl   cleaned transcript
//! snapshots.bundle   this session's snapshot commits only (compressed)
//! meta.json          ids, versions, sizes, budget check
//! labels.json        hand-label template (never overwritten)
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::hook::CV_VERSION;
use crate::redact::Redactor;
use crate::shadow::Shadow;
use crate::store::{CHECKER_DIR, latest_session, sanitize_session};

/// Target maximum size of one exported run.
pub const BUDGET_BYTES: u64 = 500 * 1024;

#[derive(Debug, Clone)]
pub struct ExportOptions {
    pub project: PathBuf,
    pub session: Option<String>,
    pub prompt_id: String,
    pub run: u32,
    pub out: PathBuf,
    /// Folder of corpus prompt files (`<prompt-id>.json`), used to pre-fill
    /// the labels template.
    pub corpus: Option<PathBuf>,
    pub allow_snapshot_secrets: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretHit {
    pub path: String,
    pub rule: &'static str,
}

#[derive(Debug, Clone)]
pub struct ExportReport {
    pub out: PathBuf,
    pub session: String,
    pub files: Vec<(String, u64)>,
    pub total_bytes: u64,
    pub over_budget: bool,
    pub snapshot_secrets: Vec<SecretHit>,
    pub labels_created: bool,
}

pub fn export(opts: &ExportOptions) -> Result<ExportReport> {
    // git runs inside the project folder, so a relative output path must
    // be made absolute first.
    let out = if opts.out.is_absolute() { opts.out.clone() } else { std::env::current_dir()?.join(&opts.out) };
    let opts = &ExportOptions { out, ..opts.clone() };
    let existed = opts.out.exists();
    let result = export_inner(opts);
    if result.is_err() && !existed {
        let _ = crate::fsx::remove_dir_all(&opts.out); // leave nothing half-written
    }
    result
}

fn export_inner(opts: &ExportOptions) -> Result<ExportReport> {
    let project = crate::fsx::canonical(&opts.project)
        .with_context(|| format!("project folder not found: {}", opts.project.display()))?;
    let checker = project.join(CHECKER_DIR);
    if !checker.is_dir() {
        bail!(
            "no recordings in {} (no .checker folder). Was the session run with tattle's hooks?",
            project.display()
        );
    }
    let session = match &opts.session {
        Some(s) => sanitize_session(s),
        None => latest_session(&checker)?.context("no recorded sessions found")?,
    };
    let session_dir = checker.join("sessions").join(&session);
    let events_src = session_dir.join("events.jsonl");
    if !events_src.is_file() {
        bail!("session {session} has no events");
    }

    let shadow = Shadow::new(&project, &checker);
    let refs = shadow.session_refs(&session)?;
    if refs.is_empty() {
        bail!("session {session} has no snapshots");
    }

    let snapshot_secrets = scan_snapshots(&shadow, &refs)?;
    if !snapshot_secrets.is_empty() && !opts.allow_snapshot_secrets {
        let list: Vec<String> = snapshot_secrets
            .iter()
            .map(|h| format!("  {} ({})", h.path, h.rule))
            .collect();
        bail!(
            "possible secrets found in the snapshots:\n{}\nNothing was exported. If these are synthetic test values, re-run with --allow-snapshot-secrets.",
            list.join("\n")
        );
    }

    if opts.out.join("events.jsonl").exists() {
        bail!(
            "{} already contains a recording; refusing to overwrite (labels may be there)",
            opts.out.display()
        );
    }
    fs::create_dir_all(&opts.out)?;

    fs::copy(&events_src, opts.out.join("events.jsonl"))?;
    let transcript_src = session_dir.join("transcript.jsonl");
    let has_transcript = transcript_src.is_file();
    if has_transcript {
        fs::write(opts.out.join("transcript.jsonl"), slim_transcript(&fs::read_to_string(&transcript_src)?))?;
    }

    let manifests_src = session_dir.join("manifests");
    if manifests_src.is_dir() {
        let dest = opts.out.join("manifests");
        fs::create_dir_all(&dest)?;
        for entry in fs::read_dir(&manifests_src)?.flatten() {
            fs::copy(entry.path(), dest.join(entry.file_name()))?;
        }
    }

    let subagents_src = session_dir.join("subagents");
    if subagents_src.is_dir() {
        let dest = opts.out.join("subagents");
        fs::create_dir_all(&dest)?;
        for entry in fs::read_dir(&subagents_src)?.flatten() {
            fs::write(dest.join(entry.file_name()), slim_transcript(&fs::read_to_string(entry.path())?))?;
        }
    }

    let bundle = opts.out.join("snapshots.bundle");
    let mut args: Vec<String> = vec!["bundle".into(), "create".into(), "-q".into()];
    args.push(bundle.to_string_lossy().into_owned());
    args.extend(refs.iter().map(|r| r.ref_name.clone()));
    shadow.git(&args).context("could not create snapshot bundle")?;

    let labels_path = opts.out.join("labels.json");
    let labels_created = !labels_path.exists();
    if labels_created {
        let template = labels_template(opts);
        fs::write(&labels_path, pretty(&template)?)?;
    }

    let events_text = fs::read_to_string(opts.out.join("events.jsonl"))?;
    let event_count = events_text.lines().filter(|l| !l.is_empty()).count();
    let agent_version = if has_transcript {
        agent_version(&opts.out.join("transcript.jsonl"))
    } else {
        None
    };

    let mut files = Vec::new();
    for name in ["events.jsonl", "transcript.jsonl", "snapshots.bundle", "labels.json"] {
        if let Ok(m) = fs::metadata(opts.out.join(name)) {
            files.push((name.to_string(), m.len()));
        }
    }
    if let Ok(entries) = fs::read_dir(opts.out.join("manifests")) {
        for entry in entries.flatten() {
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            files.push((format!("manifests/{}", entry.file_name().to_string_lossy()), len));
        }
    }
    if let Ok(entries) = fs::read_dir(opts.out.join("subagents")) {
        for entry in entries.flatten() {
            let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
            files.push((format!("subagents/{}", entry.file_name().to_string_lossy()), len));
        }
    }
    let total_bytes: u64 = files.iter().map(|(_, n)| n).sum();
    let over_budget = total_bytes > BUDGET_BYTES;

    let meta = json!({
        "schema": 1,
        "prompt_id": opts.prompt_id,
        "run": opts.run,
        "agent": "claude-code",
        "agent_version": agent_version,
        "cv_version": CV_VERSION,
        "session": session,
        "events": event_count,
        "snapshots": refs.iter().map(|r| json!({"ref": r.ref_name, "commit": r.commit})).collect::<Vec<_>>(),
        "files": files.iter().map(|(n, b)| (n.clone(), json!(b))).collect::<serde_json::Map<_, _>>(),
        "total_bytes": total_bytes,
        "budget_bytes": BUDGET_BYTES,
        "over_budget": over_budget,
        "snapshot_secrets_allowed": snapshot_secrets.iter().map(|h| json!({"path": h.path, "rule": h.rule})).collect::<Vec<_>>(),
    });
    fs::write(opts.out.join("meta.json"), pretty(&meta)?)?;

    Ok(ExportReport {
        out: opts.out.clone(),
        session,
        files,
        total_bytes,
        over_budget,
        snapshot_secrets,
        labels_created,
    })
}

/// Longest tool-input string kept in an exported transcript.
const TOOL_INPUT_KEEP: usize = 300;

/// Keep only the conversation: user/assistant/system messages with their
/// text, tool calls (name and shortened input) and tool results (id and
/// whether they failed). The events already hold tool details, and the
/// agent's private reasoning is reduced to its length. Claude Code's
/// internal bookkeeping lines (attachments, request shapes, file history)
/// are dropped. The full cleaned copy stays in the project's `.checker/`.
pub fn slim_transcript(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if let Some(slim) = slim_line(&v) {
            out.push_str(&slim.to_string());
            out.push('\n');
        }
    }
    out
}

fn slim_line(v: &Value) -> Option<Value> {
    let kind = v.get("type")?.as_str()?;
    if !matches!(kind, "user" | "assistant" | "system") {
        return None;
    }
    let msg = v.get("message")?;
    let content = match msg.get("content") {
        Some(Value::Array(blocks)) => Value::Array(blocks.iter().map(slim_block).collect()),
        Some(other) => other.clone(),
        None => Value::Null,
    };
    let mut out = serde_json::Map::new();
    for key in ["type", "uuid", "parentUuid", "timestamp", "version", "sessionId", "isSidechain"] {
        if let Some(x) = v.get(key) {
            out.insert(key.into(), x.clone());
        }
    }
    out.insert("message".into(), json!({ "role": msg.get("role"), "model": msg.get("model"), "content": content }));
    Some(Value::Object(out))
}

fn shorten(v: &Value) -> Value {
    match v {
        Value::String(s) if s.chars().count() > TOOL_INPUT_KEEP => {
            Value::String(format!("{}…", s.chars().take(TOOL_INPUT_KEEP).collect::<String>()))
        }
        Value::Object(m) => Value::Object(m.iter().map(|(k, x)| (k.clone(), shorten(x))).collect()),
        Value::Array(a) => Value::Array(a.iter().map(shorten).collect()),
        other => other.clone(),
    }
}

fn slim_block(b: &Value) -> Value {
    match b.get("type").and_then(Value::as_str) {
        Some("text") => json!({ "type": "text", "text": b.get("text") }),
        Some("thinking") => json!({ "type": "thinking", "chars": b.get("thinking").and_then(Value::as_str).map_or(0, |t| t.chars().count()) }),
        Some("tool_use") => json!({ "type": "tool_use", "id": b.get("id"), "name": b.get("name"), "input": b.get("input").map(shorten) }),
        Some("tool_result") => json!({ "type": "tool_result", "tool_use_id": b.get("tool_use_id"), "is_error": b.get("is_error") }),
        Some(other) => json!({ "type": other }),
        None => json!({}),
    }
}

fn pretty(v: &Value) -> Result<String> {
    Ok(serde_json::to_string_pretty(v)? + "\n")
}

/// Scan every file in the session's snapshots for secrets.
fn scan_snapshots(shadow: &Shadow, refs: &[crate::shadow::SnapshotRef]) -> Result<Vec<SecretHit>> {
    let commits: Vec<&str> = refs.iter().map(|r| r.commit.as_str()).collect();
    let blobs = shadow.blobs(&commits)?;
    let shas: Vec<String> = blobs.keys().cloned().collect();
    let redactor = Redactor::new();
    let mut hits = Vec::new();
    for (sha, bytes) in shadow.read_blobs(&shas)? {
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue; // binary file
        };
        let path = blobs.get(&sha).cloned().unwrap_or(sha);
        for f in redactor.find(text) {
            let hit = SecretHit {
                path: path.clone(),
                rule: f.rule,
            };
            if !hits.contains(&hit) {
                hits.push(hit);
            }
        }
    }
    hits.sort_by(|a, b| a.path.cmp(&b.path).then(a.rule.cmp(b.rule)));
    Ok(hits)
}

/// Claude Code writes its version on transcript lines.
fn agent_version(transcript: &Path) -> Option<String> {
    let text = fs::read_to_string(transcript).ok()?;
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .find_map(|v| v.get("version").and_then(Value::as_str).map(str::to_string))
}

/// Labels template, pre-filled from the corpus prompt file if there is one.
fn labels_template(opts: &ExportOptions) -> Value {
    let prompt = opts
        .corpus
        .as_ref()
        .map(|dir| dir.join(format!("{}.json", opts.prompt_id)))
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let facts: Vec<Value> = prompt
        .as_ref()
        .and_then(|p| p.get("expected_facts"))
        .and_then(Value::as_array)
        .map(|facts| {
            facts
                .iter()
                .map(|f| {
                    json!({
                        "fact_id": f.get("id").cloned().unwrap_or(Value::Null),
                        "description": f.get("description").cloned().unwrap_or(Value::Null),
                        "verdict": "",
                        "note": ""
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "prompt_id": opts.prompt_id,
        "run": opts.run,
        "labeller": "",
        "labelled_on": "",
        "_help": "verdict: met | violated | unclear. defects[].type: use the mutation names from docs/BUILD_PLAN.md section 0.5 where they fit.",
        "intent_facts": facts,
        "defects": [],
        "would_accept": null,
        "notes": ""
    })
}
