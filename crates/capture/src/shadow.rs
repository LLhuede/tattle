//! Shadow git repository: snapshots the project's files without touching the
//! user's own git history, config or hooks.
//!
//! Layout inside `<project>/.checker/`:
//! - `shadow.git/`   a separate git directory
//! - `shadow.index`  its own index file
//!
//! Each snapshot is a commit stored under `refs/cv/<session>/<seq>-<event>`,
//! with `refs/cv/<session>/last` pointing at the newest one.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

/// Files larger than this are left out of snapshots (and listed instead).
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// Folders never walked when looking for large files. The same folders are
/// excluded from snapshots by `EXCLUDE_PATTERNS`.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".checker",
    "node_modules",
    "dist",
    "build",
    ".next",
    "out",
    "coverage",
    ".turbo",
    ".cache",
    ".parcel-cache",
    ".svelte-kit",
    ".vercel",
    "target",
    "__pycache__",
    ".venv",
    "venv",
];

/// Written to `shadow.git/info/exclude`. The project's own `.gitignore`
/// files apply as well.
pub const EXCLUDE_PATTERNS: &str = "\
# Written by tattle. Never snapshot these.
.checker/
node_modules/
dist/
build/
.next/
out/
coverage/
.turbo/
.cache/
.parcel-cache/
.svelte-kit/
.vercel/
target/
__pycache__/
.venv/
venv/
.DS_Store
# Secrets
.env
.env.*
!.env.example
*.pem
*.key
*.p12
*.pfx
id_rsa*
id_ed25519*
.npmrc
.claude/settings.local.json
# Runtime data
*.db
*.db-journal
*.sqlite
*.sqlite3
*.log
";

/// Environment variables that could redirect git to the user's repository.
const GIT_ENV_TO_CLEAR: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
];

pub struct Shadow {
    pub git_dir: PathBuf,
    pub index: PathBuf,
    pub work_tree: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SnapshotInfo {
    pub commit: String,
    pub ref_name: String,
    /// Project-relative paths left out for being over `MAX_FILE_BYTES`.
    pub skipped_large: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SnapshotRef {
    pub ref_name: String,
    pub commit: String,
}

impl Shadow {
    /// Start `git repack -d` in the background and return at once. Each
    /// snapshot writes every new file as its own compressed object; packing
    /// them makes the next check read them several times faster (20,000
    /// files: 2.9 s → 0.4 s) and the folder much smaller (79 MB → 2 MB).
    /// Git allows packing while other commands read or write, so a hook
    /// running at the same moment is safe. Best effort: errors are ignored.
    pub fn pack_in_background(&self) {
        self.git_in_background(&["repack", "-d", "-q"]);
    }

    /// After old sessions' refs are deleted, start `git repack -A -d` in the
    /// background: objects nothing refers to leave the packs as loose
    /// objects, which `prune --expire` removes at a later session start once
    /// they're a day old. Never blocks session start, and never drops an
    /// object a snapshot being written right now might reuse.
    pub fn loosen_unreferenced_in_background(&self) {
        self.git_in_background(&["repack", "-A", "-d", "-q"]);
    }

    fn git_in_background(&self, args: &[&str]) {
        if !self.exists() {
            return;
        }
        let mut c = self.command();
        c.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW | DETACHED_PROCESS: no console flash, and
            // not tied to the hook's console.
            c.creation_flags(0x0800_0000 | 0x0000_0008);
        }
        let _ = c.spawn();
    }

    pub fn new(project: &Path, checker: &Path) -> Self {
        Self {
            git_dir: checker.join("shadow.git"),
            index: checker.join("shadow.index"),
            work_tree: project.to_path_buf(),
        }
    }

    pub(crate) fn command(&self) -> Command {
        let mut c = Command::new("git");
        for var in GIT_ENV_TO_CLEAR {
            c.env_remove(var);
        }
        c.arg("--git-dir")
            .arg(&self.git_dir)
            .arg("--work-tree")
            .arg(&self.work_tree)
            .args([
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.quotepath=false",
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "gc.auto=0",
                "-c",
                "commit.gpgsign=false",
            ])
            .env("GIT_INDEX_FILE", &self.index)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", "tattle")
            .env("GIT_AUTHOR_EMAIL", "tattle@localhost")
            .env("GIT_COMMITTER_NAME", "tattle")
            .env("GIT_COMMITTER_EMAIL", "tattle@localhost")
            .current_dir(&self.work_tree)
            .stdin(Stdio::null());
        c
    }

    /// Run a git command in the shadow repo and return trimmed stdout.
    pub fn git<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<S> = args.into_iter().collect();
        let shown: Vec<String> = args
            .iter()
            .map(|a| a.as_ref().to_string_lossy().into_owned())
            .collect();
        let out = self
            .command()
            .args(&args)
            .output()
            .context("could not run git (is it installed?)")?;
        if !out.status.success() {
            bail!(
                "git {} failed: {}",
                shown.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Run a git command with `input` on stdin and return trimmed stdout.
    pub fn git_with_input<I, S>(&self, args: I, input: &[u8]) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut child = self
            .command()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("could not run git")?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let input = input.to_vec();
        let writer = std::thread::spawn(move || stdin.write_all(&input));
        let out = child.wait_with_output()?;
        writer.join().expect("writer thread")?;
        if !out.status.success() {
            bail!("git failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    pub fn exists(&self) -> bool {
        self.git_dir.join("HEAD").is_file()
    }

    /// Create the shadow repo if needed. Safe to call every time.
    pub fn ensure(&self) -> Result<()> {
        if !self.exists() {
            let out = Command::new("git")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .args(["init", "-q", "--bare"])
                .arg(&self.git_dir)
                .stdin(Stdio::null())
                .output()
                .context("could not run git init")?;
            if !out.status.success() {
                bail!(
                    "git init failed: {}",
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            self.git(["config", "core.bare", "false"])?;
        }
        let info = self.git_dir.join("info");
        fs::create_dir_all(&info)?;
        let exclude = info.join("exclude");
        if fs::read_to_string(&exclude).ok().as_deref() != Some(EXCLUDE_PATTERNS) {
            fs::write(&exclude, EXCLUDE_PATTERNS)?;
        }
        Ok(())
    }

    /// Snapshot the whole project as a commit.
    pub fn snapshot(&self, session: &str, seq: u64, label: &str) -> Result<SnapshotInfo> {
        self.ensure()?;
        let skipped_large = find_large_files(&self.work_tree)?;
        let mut add: Vec<String> = vec!["add".into(), "-A".into(), "--".into(), ".".into()];
        for rel in &skipped_large {
            add.push(format!(":(exclude,literal){rel}"));
        }
        self.git(&add)?;
        let tree = self.git(["write-tree"])?;

        let last_ref = format!("refs/cv/{session}/last");
        let parent = self
            .git(["rev-parse", "--verify", "-q", format!("{last_ref}^{{commit}}").as_str()])
            .ok()
            .filter(|s| !s.is_empty());
        let label = sanitize_label(label);
        let message = format!("tattle snapshot session={session} seq={seq} event={label}");
        let mut commit_args: Vec<String> = vec!["commit-tree".into(), tree, "-m".into(), message];
        if let Some(p) = parent {
            commit_args.push("-p".into());
            commit_args.push(p);
        }
        let commit = self.git(&commit_args)?;

        let ref_name = format!("refs/cv/{session}/{seq:06}-{label}");
        self.git(["update-ref", ref_name.as_str(), commit.as_str()])?;
        self.git(["update-ref", last_ref.as_str(), commit.as_str()])?;
        Ok(SnapshotInfo {
            commit,
            ref_name,
            skipped_large,
        })
    }

    /// All snapshot refs for a session, oldest first (excludes `last`).
    pub fn session_refs(&self, session: &str) -> Result<Vec<SnapshotRef>> {
        if !self.exists() {
            return Ok(Vec::new());
        }
        let out = self.git([
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname) %(objectname)",
            format!("refs/cv/{session}/").as_str(),
        ])?;
        Ok(out
            .lines()
            .filter_map(|l| l.split_once(' '))
            .filter(|(r, _)| !r.ends_with("/last"))
            .map(|(r, c)| SnapshotRef {
                ref_name: r.to_string(),
                commit: c.to_string(),
            })
            .collect())
    }

    /// Every distinct file (blob) across the given commits, with one path
    /// for each. Sorted by path.
    pub fn blobs(&self, commits: &[&str]) -> Result<BTreeMap<String, String>> {
        let mut by_blob: BTreeMap<String, String> = BTreeMap::new();
        for commit in commits {
            let out = self.command().args(["ls-tree", "-r", "-z", commit]).output()?;
            if !out.status.success() {
                bail!("git ls-tree failed for {commit}");
            }
            for entry in out.stdout.split(|b| *b == 0).filter(|e| !e.is_empty()) {
                let entry = String::from_utf8_lossy(entry);
                // "<mode> <type> <sha>\t<path>"
                let Some((meta, path)) = entry.split_once('\t') else {
                    continue;
                };
                let mut parts = meta.split(' ');
                let (_mode, kind, sha) = (parts.next(), parts.next(), parts.next());
                if kind == Some("blob")
                    && let Some(sha) = sha
                {
                    by_blob
                        .entry(sha.to_string())
                        .or_insert_with(|| path.to_string());
                }
            }
        }
        Ok(by_blob)
    }

    /// Read blob contents in one `git cat-file --batch` call.
    pub fn read_blobs(&self, shas: &[String]) -> Result<Vec<(String, Vec<u8>)>> {
        if shas.is_empty() {
            return Ok(Vec::new());
        }
        let mut child = self
            .command()
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let input: String = shas.iter().map(|s| format!("{s}\n")).collect();
        // Write from another thread so a full output pipe can't deadlock us.
        let writer = std::thread::spawn(move || stdin.write_all(input.as_bytes()));
        let mut raw = Vec::new();
        child
            .stdout
            .take()
            .expect("piped stdout")
            .read_to_end(&mut raw)?;
        writer.join().expect("writer thread")?;
        child.wait()?;

        let mut out = Vec::new();
        let mut pos = 0;
        while pos < raw.len() {
            let Some(nl) = raw[pos..].iter().position(|b| *b == b'\n') else {
                break;
            };
            let header = String::from_utf8_lossy(&raw[pos..pos + nl]).to_string();
            pos += nl + 1;
            let parts: Vec<&str> = header.split(' ').collect();
            if parts.len() != 3 {
                continue; // "<sha> missing"
            }
            let size: usize = parts[2].parse().context("bad cat-file size")?;
            let end = (pos + size).min(raw.len());
            out.push((parts[0].to_string(), raw[pos..end].to_vec()));
            pos = end + 1; // skip trailing newline
        }
        Ok(out)
    }
}

fn sanitize_label(label: &str) -> String {
    let s: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if s.is_empty() { "event".into() } else { s }
}

/// Project-relative paths of files over `MAX_FILE_BYTES`, skipping folders
/// that are never snapshotted. Symlinks are not followed.
fn find_large_files(root: &Path) -> Result<Vec<String>> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                let name = entry.file_name();
                if !SKIP_DIRS.iter().any(|d| OsStr::new(d) == name) {
                    stack.push(path);
                }
            } else if kind.is_file()
                && entry.metadata().map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES
                && let Ok(rel) = path.strip_prefix(root)
            {
                found.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    found.sort();
    Ok(found)
}
