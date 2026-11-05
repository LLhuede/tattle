//! The Docker backend's Linux dependency pool, end to end: build the pool
//! for corpus base G2 (express, vitest, and the native `better-sqlite3`),
//! then run G2's own test suite in a no-network container with the pool
//! mounted read-only.
//!
//! Downloads packages, so it only runs with `TATTLE_TEST_INSTALL=1` and Docker.

use std::fs;
use std::path::Path;
use std::time::Duration;

use exec::linuxdeps::Options;
use exec::workspace::{Deps, resolve_dependencies, run_tests};
use exec::{Backend, Sandbox, detect_backend};

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// One pool for the tests in this file (kept between runs).
fn shared_pool() -> std::path::PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("cv-linux-deps")
}

fn enabled() -> bool {
    if capture::settings::var("TEST_INSTALL").as_deref() != Some("1") || detect_backend() != Backend::Docker {
        eprintln!("skipped: needs Docker and TATTLE_TEST_INSTALL=1 (downloads packages)");
        return false;
    }
    true
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git").arg("-C").arg(repo).args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "core.autocrlf=false"]).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `tattle verify`'s whole pipeline under Docker on a two-commit history of G2:
/// the final commit gives the extra cent to the last people instead of the
/// first, and edits the test to match (a test bent to fit a change).
#[test]
fn verify_session_under_docker() {
    if !enabled() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let repo = root.path().join("repo");
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/bases/G2"), &repo);
    git(&repo, &["init", "-q"]);
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-qm", "base"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let edit = |f: &str, from: &str, to: &str| {
        let p = repo.join(f);
        let s = fs::read_to_string(&p).unwrap();
        assert!(s.contains(from), "{f}: {from}");
        fs::write(&p, s.replace(from, to)).unwrap();
    };
    edit("src/settle.ts", "base + (i < remainder ? 1 : 0)", "base + (i >= count - remainder ? 1 : 0)");
    edit("tests/settle.test.ts", "[334, 333, 333]", "[333, 333, 334]");
    edit("tests/settle.test.ts", "[334, 334, 333]", "[333, 334, 334]");
    git(&repo, &["commit", "-qam", "change"]);
    let fin = git(&repo, &["rev-parse", "HEAD"]);

    let sources = exec::verify::DepSources { project: vec![], pool: vec![], linux: Options { root: shared_pool(), allow_install: true } };
    let rec = exec::verify::verify_session(&repo.join(".git"), Some(&base), &fin, &sources, Duration::from_secs(180)).unwrap();
    let pretty = serde_json::to_string_pretty(&rec).unwrap();
    fs::write(Path::new(env!("CARGO_TARGET_TMPDIR")).join("last-docker-record.json"), &pretty).unwrap();
    assert_eq!(rec["env"]["backend"], "docker", "{pretty}");
    for side in ["baseline", "final"] {
        assert_eq!(rec[side]["dependencies"]["strategy"], "linux-pool", "{pretty}");
        assert_eq!(rec[side]["build"]["build"]["outcome"], "passed", "{side}: {pretty}");
        assert_eq!(rec[side]["app"]["outcome"], "started", "{side}: {pretty}");
    }
    // The change also moves cents in two other tests' expectations.
    assert_eq!(rec["baseline"]["tests"]["outcome"], "passed", "{pretty}");
    assert_eq!(rec["final"]["tests"]["outcome"], "failed", "{pretty}");
    assert_eq!((rec["final"]["tests"]["runs"][0]["passed"].as_u64(), rec["final"]["tests"]["runs"][0]["failed"].as_u64()), (Some(24), Some(3)), "{pretty}");
    let c = &rec["cross"];
    let failed = |side: &str| -> Vec<String> { c[side].as_object().unwrap().iter().filter(|(_, v)| *v == "failed").map(|(k, _)| k.clone()).collect() };
    assert_eq!(c["tests_changed"], serde_json::json!(["tests/settle.test.ts"]), "{pretty}");
    // The edited test fails on the old code (P5) ...
    let edited = "tests/settle.test.ts > splitEvenly > gives leftover cents to the first people so shares add up to the total";
    assert_eq!(failed("new_tests_old_code"), vec![edited.to_string()], "{pretty}");
    // ... and its old version fails on the new code (P4), with the 3 the
    // final suite already fails.
    let mut on_new = failed("old_tests_new_code");
    on_new.sort();
    let mut want: Vec<String> = failed("final");
    want.push(edited.to_string());
    want.sort();
    assert_eq!(on_new, want, "{pretty}");
    assert_eq!(c["baseline"][edited], "passed");
}

#[test]
fn g2_runs_on_the_linux_pool() {
    if !enabled() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    let tmp = root.path().join("tmp");
    fs::create_dir_all(&tmp).unwrap();
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/bases/G2"), &work);
    let mut sb = Sandbox { work: work.clone(), tmp, deny_read: vec![], loopback: true, node_modules: None };

    // Without permission to install, nothing is downloaded.
    let empty = root.path().join("empty-pool");
    let deps = resolve_dependencies(&work, &sb, &[], &[], &Options { root: empty.clone(), allow_install: false }).unwrap();
    assert!(matches!(&deps, Deps::Unavailable(why) if why.contains("--allow-install")), "{deps:?}");
    assert!(!empty.exists());

    // With it: download, build better-sqlite3 offline, then reuse.
    let pool = shared_pool();
    let no = Options { root: pool.clone(), allow_install: false };
    let yes = Options { root: pool.clone(), allow_install: true };
    let deps = resolve_dependencies(&work, &sb, &[], &[], &yes).unwrap();
    let Deps::Pool(nm) = deps else { panic!("{deps:?}") };
    assert!(nm.join("better-sqlite3/build/Release").is_dir(), "native package built");
    let ready: serde_json::Value = serde_json::from_str(&fs::read_to_string(nm.parent().unwrap().join("ready.json")).unwrap()).unwrap();
    assert_eq!(ready["native"][0], "better-sqlite3");
    assert_eq!(resolve_dependencies(&work, &sb, &[], &[], &no).unwrap(), Deps::Pool(nm.clone()), "reused without install");

    sb.node_modules = Some(nm.clone());
    let run = run_tests(&sb, &work, Duration::from_secs(180)).unwrap().unwrap();
    let text = format!("{}\n{}", run.result.stdout, run.result.stderr);
    assert!(run.result.ok(), "{text}");
    assert_eq!(run.summary.map(|s| (s.passed, s.failed)), Some((27, 0)), "{text}");
    // The pool stayed read-only: the vitest cache went to scratch space.
    assert!(!nm.join(".vite/vitest").exists());
    assert!(work.join("node_modules").read_dir().unwrap().next().is_none(), "nothing written into the copy's mount point");

}
