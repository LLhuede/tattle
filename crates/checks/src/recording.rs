//! Loads a recording and gives read access to its events, transcript and
//! code snapshots. Two sources:
//! - an exported recording (the folder `tattle record export` writes)
//! - a live session in a project's `.checker/` folder, read in place while
//!   Claude Code is still running (the plugin's end-of-turn check)

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use capture::shadow::Shadow;
use serde_json::Value;

/// One code snapshot, taken at a hook event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Index into `Recording::events`.
    pub index: usize,
    pub seq: u64,
    pub event: String,
    pub commit: String,
    pub ref_name: String,
}

/// A file that differs between two snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// `A` added, `M` modified, `D` deleted, `T` type changed.
    pub status: char,
    pub path: String,
}

pub struct Recording {
    pub dir: PathBuf,
    pub events: Vec<Value>,
    pub transcript: Vec<Value>,
    pub meta: Value,
    pub labels: Option<Value>,
    /// Manifests the agent wrote (Phase 6b), oldest first.
    pub manifests: Vec<Value>,
    /// Where checks may keep work between runs (live sessions only).
    pub cache_dir: Option<PathBuf>,
    git: Shadow,
    _tmp: tempfile::TempDir,
}

pub fn read_jsonl(path: &Path) -> Result<Vec<Value>> {
    let text = fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).with_context(|| format!("{}: line {}", path.display(), i + 1)))
        .collect()
}

/// Like [`read_jsonl`], but drops lines that don't parse (a line another
/// process is still writing).
fn read_jsonl_lenient(path: &Path) -> Result<Vec<Value>> {
    let text = fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    Ok(text.lines().filter(|l| !l.trim().is_empty()).filter_map(|l| serde_json::from_str(l).ok()).collect())
}

fn read_json(path: &Path) -> Result<Option<Value>> {
    match fs::read_to_string(path) {
        Ok(t) => Ok(Some(serde_json::from_str(&t).with_context(|| format!("{}", path.display()))?)),
        Err(_) => Ok(None),
    }
}

impl Recording {
    pub fn load(dir: &Path) -> Result<Self> {
        let events = read_jsonl(&dir.join("events.jsonl"))?;
        let transcript_path = dir.join("transcript.jsonl");
        let transcript = if transcript_path.is_file() {
            read_jsonl(&transcript_path)?
        } else {
            Vec::new()
        };
        let meta = read_json(&dir.join("meta.json"))?.unwrap_or(Value::Null);
        let labels = read_json(&dir.join("labels.json"))?;
        let mut manifest_files: Vec<PathBuf> = fs::read_dir(dir.join("manifests"))
            .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect())
            .unwrap_or_default();
        manifest_files.sort();
        let manifests = manifest_files.iter().filter_map(|p| read_json(p).ok().flatten()).collect();

        let tmp = tempfile::tempdir()?;
        let git = Shadow {
            git_dir: tmp.path().join("repo.git"),
            index: tmp.path().join("index"),
            work_tree: tmp.path().to_path_buf(),
        };
        git.ensure()?;
        let bundle = dir.join("snapshots.bundle");
        if bundle.is_file() {
            let bundle = capture::fsx::canonical(&bundle)?;
            git.git([
                "fetch".as_ref(),
                "-q".as_ref(),
                bundle.as_os_str(),
                "refs/cv/*:refs/cv/*".as_ref(),
            ])
            .context("could not read snapshots.bundle")?;
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            events,
            transcript,
            meta,
            labels,
            manifests,
            git,
            cache_dir: None,
            _tmp: tmp,
        })
    }

    /// A session that is still being recorded, read in place from
    /// `<project>/.checker/sessions/<session>/`. The checker sees exactly
    /// what an export would give it (the transcript is slimmed the same
    /// way), but nothing is copied or scanned, so it is fast enough to run
    /// at the end of every turn. Snapshots are read from the project's
    /// shadow repo through git alternates; nothing there is changed.
    pub fn load_live(project: &Path, session: &str) -> Result<Self> {
        let checker = project.join(capture::store::CHECKER_DIR);
        let dir = checker.join("sessions").join(session);
        // Other hooks may be appending while this runs: skip a torn last line.
        let events = read_jsonl_lenient(&dir.join("events.jsonl"))?;
        let transcript = match fs::read_to_string(dir.join("transcript.jsonl")) {
            Ok(text) => capture::export::slim_transcript(&text)
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect(),
            Err(_) => Vec::new(),
        };
        let mut manifest_files: Vec<PathBuf> = fs::read_dir(dir.join("manifests"))
            .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "json")).collect())
            .unwrap_or_default();
        manifest_files.sort();
        let manifests = manifest_files.iter().filter_map(|p| read_json(p).ok().flatten()).collect();

        let cache_dir = dir.clone();
        let tmp = tempfile::tempdir()?;
        let git = Shadow {
            git_dir: tmp.path().join("repo.git"),
            index: tmp.path().join("index"),
            work_tree: tmp.path().to_path_buf(),
        };
        git.ensure()?;
        let objects = checker.join("shadow.git").join("objects");
        if objects.is_dir() {
            let alternates = git.git_dir.join("objects").join("info");
            fs::create_dir_all(&alternates)?;
            fs::write(alternates.join("alternates"), format!("{}\n", capture::fsx::canonical(&objects)?.display()))?;
        }
        Ok(Self {
            dir,
            events,
            transcript,
            meta: Value::Null,
            labels: None,
            manifests,
            git,
            cache_dir: Some(cache_dir),
            _tmp: tmp,
        })
    }

    /// Drop the turn still in progress: the last prompt the person typed,
    /// when Claude hasn't finished it yet (no `Stop` after it). `tattle report`
    /// runs inside such a turn (the person just typed `/tattle:report`), and
    /// half a turn would read as a recording gap. Returns whether a turn was
    /// dropped.
    pub fn drop_open_turn(&mut self) -> bool {
        let is_human = |e: &Value| {
            e["event"] == "UserPromptSubmit"
                && !e["payload"]["prompt"].as_str().is_some_and(|p| p.trim_start().starts_with("<task-notification>"))
        };
        let Some(last) = self.events.iter().rposition(is_human) else { return false };
        if self.events[last..].iter().any(|e| e["event"] == "Stop") {
            return false;
        }
        self.events.truncate(last);
        true
    }

    /// Drop every turn whose prompt the person typed matches `is_command`
    /// (the plugin's own commands: `/tattle:mute …`, `/tattle:report`), from the
    /// prompt through its `Stop`. A request about tattle's settings isn't work
    /// to check: its words ("mute claims.tests_pass_unverified") read as a
    /// reference to code that doesn't exist. Later events' sequence numbers
    /// move down by what was dropped, so a real gap is still seen. Returns
    /// how many turns were dropped.
    pub fn drop_turns(&mut self, is_command: impl Fn(&str) -> bool) -> usize {
        let mut kept = Vec::with_capacity(self.events.len());
        let (mut dropped, mut skipping, mut turns) = (0u64, false, 0usize);
        for mut e in std::mem::take(&mut self.events) {
            if e["event"] == "UserPromptSubmit" {
                skipping = e["payload"]["prompt"].as_str().is_some_and(|p| is_command(p.trim_start()));
                turns += usize::from(skipping);
            }
            if skipping {
                dropped += 1;
                if e["event"] == "Stop" {
                    skipping = false;
                }
                continue;
            }
            if let Some(seq) = e["seq"].as_u64() {
                e["seq"] = Value::from(seq - dropped.min(seq));
            }
            kept.push(e);
        }
        self.events = kept;
        turns
    }

    /// Number of prompts the person typed (task notices don't count).
    pub fn human_turns(&self) -> usize {
        crate::human_prompts(self).len()
    }

    /// Short display name, e.g. `G1/run1`.
    pub fn name(&self) -> String {
        let run = self.dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let parent = self
            .dir
            .parent()
            .and_then(|p| p.file_name())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{parent}/{run}")
    }

    /// Git access to the snapshots (read and write; it's a private copy).
    pub fn git(&self) -> &Shadow {
        &self.git
    }

    pub fn snapshots(&self) -> Vec<Snapshot> {
        self.events
            .iter()
            .enumerate()
            .filter_map(|(index, e)| {
                let snap = e.get("snapshot")?;
                Some(Snapshot {
                    index,
                    seq: e.get("seq")?.as_u64()?,
                    event: e.get("event")?.as_str()?.to_string(),
                    commit: snap.get("commit")?.as_str()?.to_string(),
                    ref_name: snap.get("ref")?.as_str()?.to_string(),
                })
            })
            .collect()
    }

    /// The code before the session started.
    pub fn baseline(&self) -> Option<Snapshot> {
        self.snapshots().into_iter().next()
    }

    /// The code when the agent finished: the last `Stop` snapshot, or else
    /// the last snapshot.
    pub fn final_snapshot(&self) -> Option<Snapshot> {
        let snaps = self.snapshots();
        snaps
            .iter()
            .rev()
            .find(|s| s.event == "Stop")
            .or(snaps.last())
            .cloned()
    }

    /// Several files' contents at a snapshot in two git calls. Missing
    /// files are left out of the result.
    pub fn read_files(&self, commit: &str, paths: &[String]) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        let Ok(listing) = self.git.git(["ls-tree", "-r", commit]) else { return out };
        let wanted: std::collections::BTreeSet<&str> = paths.iter().map(String::as_str).collect();
        let mut sha_to_paths: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
        for line in listing.lines() {
            // "<mode> blob <sha>\t<path>"
            let Some((meta, path)) = line.split_once('\t') else { continue };
            let mut parts = meta.split(' ');
            if let (Some(_), Some("blob"), Some(sha)) = (parts.next(), parts.next(), parts.next())
                && wanted.contains(path)
            {
                sha_to_paths.entry(sha.to_string()).or_default().push(path.to_string());
            }
        }
        let shas: Vec<String> = sha_to_paths.keys().cloned().collect();
        if let Ok(blobs) = self.git.read_blobs(&shas) {
            for (sha, bytes) in blobs {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                for p in sha_to_paths.get(&sha).into_iter().flatten() {
                    out.insert(p.clone(), text.clone());
                }
            }
        }
        out
    }

    /// A file's exact contents at a snapshot (lossy UTF-8), if it exists.
    pub fn read_file(&self, commit: &str, path: &str) -> Option<String> {
        let out = self
            .git
            .git(["rev-parse", &format!("{commit}:{path}")])
            .ok()?;
        let blobs = self.git.read_blobs(&[out]).ok()?;
        blobs.into_iter().next().map(|(_, b)| String::from_utf8_lossy(&b).into_owned())
    }

    pub fn list_files(&self, commit: &str) -> Result<Vec<String>> {
        let out = self.git.git(["ls-tree", "-r", "--name-only", commit])?;
        Ok(out.lines().map(str::to_string).collect())
    }

    /// Code files at `commit` that match a `git grep` (e.g. `["-w", "-e",
    /// "name"]`). Searches JS/TS files only.
    pub fn grep_code(&self, commit: &str, args: &[&str]) -> Vec<String> {
        let mut full: Vec<&str> = vec!["grep", "-l", "-I"];
        full.extend_from_slice(args);
        full.push(commit);
        full.extend_from_slice(&["--", "*.ts", "*.tsx", "*.js", "*.jsx", "*.mjs", "*.cjs", "*.mts", "*.cts"]);
        // `git grep` exits 1 when nothing matches.
        let Ok(out) = self.git.git(&full) else { return Vec::new() };
        let prefix = format!("{commit}:");
        out.lines().map(|l| l.strip_prefix(&prefix).unwrap_or(l).to_string()).collect()
    }

    /// Files that differ between two snapshots, sorted by path.
    pub fn changed_files(&self, from: &str, to: &str) -> Result<Vec<Change>> {
        let out = self.git.git(["diff-tree", "-r", "--no-renames", "--name-status", from, to])?;
        let mut changes: Vec<Change> = out
            .lines()
            .filter_map(|l| {
                let (status, path) = l.split_once('\t')?;
                Some(Change {
                    status: status.chars().next()?,
                    path: path.to_string(),
                })
            })
            .collect();
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(events: Vec<Value>) -> Recording {
        let tmp = tempfile::tempdir().unwrap();
        let git = Shadow { git_dir: tmp.path().join("g"), index: tmp.path().join("i"), work_tree: tmp.path().to_path_buf() };
        Recording { dir: tmp.path().to_path_buf(), events, transcript: vec![], meta: Value::Null, labels: None, manifests: vec![], cache_dir: None, git, _tmp: tmp }
    }

    fn prompt(p: &str) -> Value {
        json!({"event": "UserPromptSubmit", "payload": {"prompt": p}})
    }

    #[test]
    fn drops_the_plugins_own_commands() {
        let stop = |seq: u64| json!({"seq": seq, "event": "Stop"});
        let p = |seq: u64, text: &str| {
            let mut e = prompt(text);
            e["seq"] = json!(seq);
            e
        };
        let mut r = rec(vec![
            p(1, "add mul"), stop(2),
            p(3, "/tattle:mute claims.tests_pass_unverified"), stop(4),
            p(5, "add div"), json!({"seq": 7, "event": "PostToolUse"}), stop(8),
        ]);
        assert_eq!(r.drop_turns(|t| t.starts_with("/tattle:")), 1);
        let seqs: Vec<u64> = r.events.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
        assert_eq!(seqs, [1, 2, 3, 5, 6], "later events move down; the real gap (6 missing) stays");
        assert_eq!(r.human_turns(), 2);
    }

    #[test]
    fn drops_only_an_unfinished_last_turn() {
        let stop = json!({"event": "Stop"});
        // `/tattle:report` typed after a finished turn: its own turn is dropped.
        let mut r = rec(vec![prompt("add mul"), stop.clone(), prompt("/tattle:report")]);
        assert!(r.drop_open_turn());
        assert_eq!(r.events.len(), 2);
        assert_eq!(r.human_turns(), 1);
        // Finished: nothing dropped.
        let mut r = rec(vec![prompt("add mul"), stop.clone()]);
        assert!(!r.drop_open_turn());
        assert_eq!(r.events.len(), 2);
        // A task notice after the last stop isn't the person's turn.
        let mut r = rec(vec![prompt("add mul"), stop, prompt("<task-notification>done</task-notification>")]);
        assert!(!r.drop_open_turn());
    }
}
