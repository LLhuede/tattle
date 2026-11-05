//! On-disk layout of captured data, inside the project being watched:
//!
//! ```text
//! <project>/.checker/
//!   .gitignore                 "*" — the folder ignores itself, so the
//!                              user's .gitignore never needs changing
//!   errors.log                 hook failures (the hook itself never fails)
//!   shadow.git/, shadow.index  snapshots (see shadow.rs)
//!   sessions/<session-id>/
//!     events.jsonl             one cleaned event per line
//!     transcript.jsonl         cleaned copy of the agent transcript
//!     seq                      last sequence number
//!     lock                     serialises concurrent hook calls
//! ```

use std::fs::{self, File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use crate::text::sha256_hex;

pub const CHECKER_DIR: &str = ".checker";

/// Create `<project>/.checker` (owner-only) with its self-ignoring
/// `.gitignore`.
pub fn ensure_checker(project: &Path) -> Result<PathBuf> {
    let dir = project.join(CHECKER_DIR);
    fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    #[cfg(unix)]
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    let gitignore = dir.join(".gitignore");
    if !gitignore.exists() {
        fs::write(&gitignore, "*\n")?;
    }
    Ok(dir)
}

/// Session ids are used in file and ref names, so only allow safe
/// characters. Anything else is replaced by a stable hash.
pub fn sanitize_session(id: &str) -> String {
    let safe = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if safe {
        id.to_string()
    } else {
        format!("s-{}", &sha256_hex(id.as_bytes())[..16])
    }
}

pub struct Session {
    pub id: String,
    pub dir: PathBuf,
}

impl Session {
    pub fn open(checker: &Path, id: &str) -> Result<Self> {
        let dir = checker.join("sessions").join(id);
        fs::create_dir_all(&dir)?;
        Ok(Self {
            id: id.to_string(),
            dir,
        })
    }

    pub fn events_path(&self) -> PathBuf {
        self.dir.join("events.jsonl")
    }

    pub fn transcript_path(&self) -> PathBuf {
        self.dir.join("transcript.jsonl")
    }

    /// Exclusive lock held for the whole hook call. Released on drop.
    pub fn lock(&self) -> Result<File> {
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join("lock"))?;
        f.lock().context("could not lock session")?;
        Ok(f)
    }

    /// Next sequence number. Call only while holding `lock()`.
    pub fn next_seq(&self) -> Result<u64> {
        let path = self.dir.join("seq");
        let current: u64 = fs::read_to_string(&path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        let next = current + 1;
        fs::write(&path, next.to_string())?;
        Ok(next)
    }

    /// Append one event as a JSON line (file is owner read/write only).
    pub fn append_event(&self, event: &Value) -> Result<()> {
        let mut line = serde_json::to_string(event)?;
        line.push('\n');
        write_private(&self.events_path(), line.as_bytes(), true)
    }
}

/// Write (or append to) a file readable only by its owner.
pub fn write_private(path: &Path, bytes: &[u8], append: bool) -> Result<()> {
    let mut opts = OpenOptions::new();
    opts.create(true);
    if append {
        opts.append(true);
    } else {
        opts.write(true).truncate(true);
    }
    #[cfg(unix)]
    opts.mode(0o600);
    let mut f = opts
        .open(path)
        .with_context(|| format!("could not open {}", path.display()))?;
    f.write_all(bytes)?;
    Ok(())
}

/// The session whose event log changed most recently.
pub fn latest_session(checker: &Path) -> Result<Option<String>> {
    let sessions = checker.join("sessions");
    let Ok(entries) = fs::read_dir(&sessions) else {
        return Ok(None);
    };
    let mut best: Option<(std::time::SystemTime, String)> = None;
    for entry in entries.flatten() {
        let events = entry.path().join("events.jsonl");
        if let Ok(modified) = fs::metadata(&events).and_then(|m| m.modified()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                best = Some((modified, name));
            }
        }
    }
    Ok(best.map(|(_, name)| name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_are_sanitised() {
        assert_eq!(sanitize_session("abc-123_DEF"), "abc-123_DEF");
        let hashed = sanitize_session("../../etc");
        assert!(hashed.starts_with("s-") && hashed.len() == 18);
        assert_eq!(hashed, sanitize_session("../../etc"));
        assert!(sanitize_session("").starts_with("s-"));
    }

    #[test]
    fn checker_dir_ignores_itself() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = ensure_checker(tmp.path()).unwrap();
        assert_eq!(fs::read_to_string(dir.join(".gitignore")).unwrap(), "*\n");
    }

    #[test]
    fn sequence_numbers_increase() {
        let tmp = tempfile::tempdir().unwrap();
        let s = Session::open(tmp.path(), "abc").unwrap();
        let _l = s.lock().unwrap();
        assert_eq!(s.next_seq().unwrap(), 1);
        assert_eq!(s.next_seq().unwrap(), 2);
    }
}
