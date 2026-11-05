//! Keeping `.checker/` small without the person ever thinking about it
//! (plugin mode only; research sandboxes keep everything).
//!
//! - Old sessions are deleted at the start of a new one: anything older than
//!   [`MAX_AGE`], and anything beyond the newest [`KEEP_SESSIONS`].
//! - Their snapshots are removed from the shadow repo: a background
//!   `repack -A` moves what nothing refers to out of the packs, and `prune`
//!   removes those objects at a later session start once they are a day
//!   old, so a snapshot another session is writing is never touched and
//!   session start never waits.
//! - `errors.log` is rotated once it passes [`MAX_LOG_BYTES`].

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::Result;

use crate::shadow::Shadow;

/// Sessions kept per project.
pub const KEEP_SESSIONS: usize = 30;
/// Sessions older than this are deleted.
pub const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);
/// `errors.log` is rotated to `errors.log.old` past this size.
pub const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Delete old sessions. `current` is always kept, plus the newest
/// [`KEEP_SESSIONS`] others. Returns the ids removed, sorted.
pub fn prune_sessions(project: &Path, checker: &Path, current: &str, now: SystemTime) -> Result<Vec<String>> {
    prune_sessions_with(project, checker, current, now, true)
}

/// [`prune_sessions`]; `background: false` repacks in place (tests).
fn prune_sessions_with(project: &Path, checker: &Path, current: &str, now: SystemTime, background: bool) -> Result<Vec<String>> {
    let Ok(entries) = fs::read_dir(checker.join("sessions")) else { return Ok(Vec::new()) };
    let mut sessions: Vec<(SystemTime, String)> = entries
        .flatten()
        .filter(|e| e.path().is_dir() && e.file_name() != current)
        .map(|e| {
            let events = e.path().join("events.jsonl");
            let modified = fs::metadata(&events)
                .or_else(|_| fs::metadata(e.path()))
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (modified, e.file_name().to_string_lossy().into_owned())
        })
        .collect();
    // Newest first; ties broken by name so the result is deterministic.
    sessions.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut removed = Vec::new();
    for (i, (modified, id)) in sessions.iter().enumerate() {
        let too_old = now.duration_since(*modified).is_ok_and(|age| age > MAX_AGE);
        if i < KEEP_SESSIONS && !too_old {
            continue;
        }
        fs::remove_dir_all(checker.join("sessions").join(id))?;
        removed.push(id.clone());
    }
    if removed.is_empty() {
        return Ok(removed);
    }
    let shadow = Shadow::new(project, checker);
    if shadow.exists() {
        let mut deletes = String::new();
        for id in &removed {
            for r in shadow.session_refs(id)? {
                deletes.push_str(&format!("delete {}\n", r.ref_name));
            }
            deletes.push_str(&format!("delete refs/cv/{id}/last\n"));
        }
        shadow.git_with_input(["update-ref", "--stdin"], deletes.as_bytes())?;
        // Snapshots are packed after each check, and `prune` only removes
        // loose objects. So `repack -A` turns what nothing refers to back
        // into loose objects (in the background: session start never
        // waits), and `prune` removes them a day later, at a later session
        // start. Nothing another session is writing right now is dropped.
        shadow.git(["prune", "--expire=1.day.ago"])?;
        if background {
            shadow.loosen_unreferenced_in_background();
        } else {
            shadow.git(["repack", "-A", "-d", "-q"])?;
        }
    }
    removed.sort();
    Ok(removed)
}

/// Move `errors.log` aside once it gets large (keeps one old copy).
pub fn rotate_log(checker: &Path) {
    let log = checker.join("errors.log");
    if fs::metadata(&log).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = fs::rename(&log, checker.join("errors.log.old"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Session, ensure_checker};

    fn touch_session(checker: &Path, id: &str) {
        let s = Session::open(checker, id).unwrap();
        s.append_event(&serde_json::json!({"seq": 1})).unwrap();
    }

    #[test]
    fn keeps_recent_sessions_and_the_current_one() {
        let tmp = tempfile::tempdir().unwrap();
        let checker = ensure_checker(tmp.path()).unwrap();
        for i in 0..(KEEP_SESSIONS + 3) {
            touch_session(&checker, &format!("s{i:03}"));
        }
        let now = SystemTime::now();
        let removed = prune_sessions(tmp.path(), &checker, "s000", now).unwrap();
        assert_eq!(removed.len(), 2, "{removed:?}");
        assert!(!removed.contains(&"s000".to_string()));
        assert!(checker.join("sessions/s000").is_dir());
        assert_eq!(fs::read_dir(checker.join("sessions")).unwrap().count(), KEEP_SESSIONS + 1);
        // Far in the future, everything but the current session is too old.
        let later = now + MAX_AGE + Duration::from_secs(60);
        let removed = prune_sessions(tmp.path(), &checker, "s000", later).unwrap();
        assert_eq!(removed.len(), KEEP_SESSIONS);
        assert_eq!(fs::read_dir(checker.join("sessions")).unwrap().count(), 1);
    }

    #[test]
    fn removes_old_snapshots_too() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), "x").unwrap();
        let checker = ensure_checker(tmp.path()).unwrap();
        let shadow = Shadow::new(tmp.path(), &checker);
        for id in ["old", "new"] {
            touch_session(&checker, id);
            shadow.snapshot(id, 1, "SessionStart").unwrap();
        }
        let later = SystemTime::now() + MAX_AGE + Duration::from_secs(60);
        prune_sessions(tmp.path(), &checker, "new", later).unwrap();
        assert!(shadow.session_refs("old").unwrap().is_empty());
        assert_eq!(shadow.session_refs("new").unwrap().len(), 1);
    }

    #[test]
    fn a_deleted_sessions_files_leave_the_disk_even_when_packed() {
        // Snapshots are packed after each check; `git prune` alone only
        // removes loose objects, so deleted sessions' files stayed in the
        // pack (found by the README security review, 2026-11-04).
        let tmp = tempfile::tempdir().unwrap();
        let checker = ensure_checker(tmp.path()).unwrap();
        let shadow = Shadow::new(tmp.path(), &checker);
        fs::write(tmp.path().join("secret.txt"), "only in the old session").unwrap();
        touch_session(&checker, "old");
        shadow.snapshot("old", 1, "SessionStart").unwrap();
        let blob = shadow.git(["rev-parse", "refs/cv/old/last:secret.txt"]).unwrap();
        fs::write(tmp.path().join("secret.txt"), "now").unwrap();
        touch_session(&checker, "new");
        shadow.snapshot("new", 1, "SessionStart").unwrap();
        shadow.git(["repack", "-d", "-q"]).unwrap();
        let later = SystemTime::now() + MAX_AGE + Duration::from_secs(60);
        prune_sessions_with(tmp.path(), &checker, "new", later, false).unwrap();
        // A later session start prunes the loosened objects once a day old.
        shadow.git(["prune", "--expire=now"]).unwrap();
        assert!(shadow.git(["cat-file", "-e", blob.as_str()]).is_err(), "old file contents still stored");
        assert!(shadow.git(["rev-parse", "refs/cv/new/last:secret.txt"]).is_ok(), "the current session is kept");
    }

    #[test]
    fn rotates_a_large_log() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("errors.log"), vec![b'x'; MAX_LOG_BYTES as usize + 1]).unwrap();
        rotate_log(tmp.path());
        assert!(!tmp.path().join("errors.log").exists());
        assert!(tmp.path().join("errors.log.old").exists());
    }
}
