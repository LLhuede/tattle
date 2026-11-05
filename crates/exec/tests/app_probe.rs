//! Gate 4b.3: the app-start probe (P3) on real projects. Every reference
//! server and the Opus flagship must start; the two Haiku flagship runs
//! (broken at turn 16) must not. Projects missing from this machine are
//! skipped.

use std::fs;
use std::path::{Path, PathBuf};

use exec::verify::app_probe;
use exec::{Backend, Sandbox, detect_backend, link_node_modules};

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        if name == "node_modules" || name == ".git" || name == ".checker" || name == ".claude" {
            continue;
        }
        let dest = to.join(&name);
        let ft = entry.file_type().unwrap();
        if ft.is_dir() {
            copy_dir(&entry.path(), &dest);
        } else if ft.is_file() {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// Copy the project into a sandbox, link its own node_modules, probe.
fn probe(project: &Path) -> Option<String> {
    if detect_backend() == Backend::Unavailable || !project.join("package.json").is_file() {
        eprintln!("skipped: {}", project.display());
        return None;
    }
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    let tmp = root.path().join("tmp");
    fs::create_dir_all(&tmp).unwrap();
    copy_dir(project, &work);
    if project.join("node_modules").is_dir() {
        link_node_modules(&project.join("node_modules"), &work).unwrap();
    }
    let sb = Sandbox { work: work.clone(), tmp, deny_read: vec![], loopback: true, node_modules: None };
    let v = app_probe(&sb, &work).unwrap()?;
    eprintln!("{}: {}", project.display(), v);
    Some(v["outcome"].as_str().unwrap().to_string())
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn big(name: &str) -> PathBuf {
    capture::fsx::home_dir().unwrap().join("Code/cv-sandbox/big").join(name)
}

#[test]
fn reference_servers_start() {
    for r in ["A-flagship", "B-breadth/b1-notes-api", "B-breadth/b4-job-worker", "B-breadth/b6-inventory-api"] {
        let dir = repo().join("bigtests").join(r).join("reference");
        if let Some(outcome) = probe(&dir) {
            assert_eq!(outcome, "started", "{r}");
        }
    }
}

#[test]
fn the_opus_flagship_starts() {
    if let Some(outcome) = probe(&big("A1-claude-opus-5-5-run1")) {
        assert_eq!(outcome, "started");
    }
}

#[test]
fn the_broken_haiku_flagships_do_not_start() {
    for run in ["A1-claude-haiku-4-5-run1", "A1-claude-haiku-4-5-run2"] {
        if let Some(outcome) = probe(&big(run)) {
            assert!(outcome == "crashed" || outcome == "no-listen" || outcome == "error-response", "{run}: {outcome}");
        }
    }
}

#[test]
fn libraries_are_not_servers() {
    let lib = repo().join("bigtests/B-breadth/b3-money-lib/reference");
    if lib.is_dir() {
        assert!(!exec::verify::is_server(&lib));
    }
}
