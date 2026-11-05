//! Small file-system helpers that behave the same on macOS, Linux and
//! Windows.

use std::io;
use std::path::{Path, PathBuf};

/// The user's home folder. Windows keeps it in `USERPROFILE` (`HOME` is
/// only set by some shells, such as Git Bash).
pub fn home_dir() -> Option<PathBuf> {
    home_dirs().into_iter().next()
}

/// Every folder the environment names as home, main one first. On Windows
/// `HOME` (from Git Bash, or set by hand) can differ from `USERPROFILE`, and
/// either is a home folder.
pub fn home_dirs() -> Vec<PathBuf> {
    let keys: &[&str] = if cfg!(windows) { &["USERPROFILE", "HOME"] } else { &["HOME"] };
    let mut out: Vec<PathBuf> = Vec::new();
    for k in keys {
        if let Some(v) = std::env::var_os(k).filter(|v| !v.is_empty()) {
            let p = PathBuf::from(v);
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

/// `fs::canonicalize` without Windows' `\\?\` prefix (`\\?\C:\x` becomes
/// `C:\x`, `\\?\UNC\server\share` becomes `\\server\share`). Git and most
/// tools can't use the prefixed form.
pub fn canonical(p: &Path) -> io::Result<PathBuf> {
    Ok(strip_verbatim(std::fs::canonicalize(p)?))
}

/// `fs::remove_dir_all` that also works on Windows when the tree holds
/// read-only files (git makes its object files read-only).
pub fn remove_dir_all(p: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(p) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            clear_readonly(p);
            std::fs::remove_dir_all(p)
        }
        other => other,
    }
}

#[allow(clippy::permissions_set_readonly_false)] // only on Windows-style errors, inside our own temp folders
fn clear_readonly(p: &Path) {
    let Ok(meta) = std::fs::symlink_metadata(p) else { return };
    if meta.is_dir() {
        for entry in std::fs::read_dir(p).into_iter().flatten().flatten() {
            clear_readonly(&entry.path());
        }
    }
    let mut perms = meta.permissions();
    if perms.readonly() {
        perms.set_readonly(false);
        let _ = std::fs::set_permissions(p, perms);
    }
}

/// The ways a folder's path can be written: as given, and for Windows drive
/// paths also with forward slashes, Git Bash style (`/c/Users/a`) and either
/// drive-letter case. Placeholders replace all of them.
pub fn path_spellings(text: &str) -> Vec<String> {
    let mut out = vec![text.to_string()];
    let b = text.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/') {
        let rest = &text[2..];
        for drive in [text[..1].to_ascii_uppercase(), text[..1].to_ascii_lowercase()] {
            out.push(format!("{drive}:{rest}"));
            out.push(format!("{drive}:{}", rest.replace('\\', "/")));
        }
        out.push(format!("/{}{}", text[..1].to_ascii_lowercase(), rest.replace('\\', "/")));
    }
    let mut seen = std::collections::BTreeSet::new();
    out.retain(|s| seen.insert(s.clone()));
    out
}

pub fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        return PathBuf::from(rest);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_prefix_is_removed() {
        assert_eq!(strip_verbatim(PathBuf::from(r"\\?\C:\Users\a\proj")), PathBuf::from(r"C:\Users\a\proj"));
        assert_eq!(strip_verbatim(PathBuf::from(r"\\?\UNC\srv\share\p")), PathBuf::from(r"\\srv\share\p"));
        assert_eq!(strip_verbatim(PathBuf::from("/home/a/proj")), PathBuf::from("/home/a/proj"));
        // a verbatim path that isn't a drive path is kept as is
        assert_eq!(strip_verbatim(PathBuf::from(r"\\?\GLOBALROOT\x")), PathBuf::from(r"\\?\GLOBALROOT\x"));
    }

    #[test]
    fn windows_path_spellings() {
        let s = path_spellings(r"C:\Users\ann\proj");
        for want in [r"C:\Users\ann\proj", r"c:\Users\ann\proj", "C:/Users/ann/proj", "c:/Users/ann/proj", "/c/Users/ann/proj"] {
            assert!(s.contains(&want.to_string()), "{want} in {s:?}");
        }
        assert_eq!(path_spellings("/home/ann/proj"), vec!["/home/ann/proj"]);
    }
}
