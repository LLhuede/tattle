//! Escape tests for the execution sandbox (Phase 4b gate 4b.0). Every test
//! tries to get out and must fail. They run on whichever backend this
//! machine has (the macOS sandbox, or Docker) and are skipped without one.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use exec::{Backend, Sandbox, detect_backend, run};

/// The escape tests run one at a time: the process-bomb test would starve a
/// test suite running alongside it (seen in `cargo test`).
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Env {
    _serial: std::sync::MutexGuard<'static, ()>,
    _root: tempfile::TempDir,
    sb: Sandbox,
    outside: PathBuf,
    secret: PathBuf,
}

fn setup(loopback: bool) -> Option<Env> {
    if detect_backend() == Backend::Unavailable {
        eprintln!("skipped: no sandbox (macOS) or Docker on this machine");
        return None;
    }
    let serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    let tmp = root.path().join("tmp");
    let outside = root.path().join("outside");
    let secret = root.path().join("secret");
    for d in [&work, &tmp, &outside, &secret] {
        fs::create_dir_all(d).unwrap();
    }
    fs::write(secret.join("id_rsa"), "PRIVATE KEY").unwrap();
    let sb = Sandbox { work, tmp, deny_read: vec![secret.clone()], loopback, node_modules: None };
    Some(Env { _serial: serial, _root: root, sb, outside, secret })
}

fn sh(e: &Env, script: &str, timeout: u64) -> exec::RunResult {
    run(&e.sb, &e.sb.work, &["/bin/sh", "-c", script], &[], Duration::from_secs(timeout)).unwrap()
}

/// bash, for `exec -a` (Debian's `/bin/sh` is dash).
fn bash(e: &Env, script: &str, timeout: u64) -> exec::RunResult {
    run(&e.sb, &e.sb.work, &["/bin/bash", "-c", script], &[], Duration::from_secs(timeout)).unwrap()
}

/// Processes on this machine whose command line contains `marker`, or None
/// when that can't be seen from here (no `pgrep`; Docker Desktop's VM).
fn survivors(marker: &str) -> Option<String> {
    let out = std::process::Command::new("pgrep").args(["-f", marker]).output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[test]
fn writes_inside_the_copy_work() {
    let Some(e) = setup(false) else { return };
    let r = sh(&e, "echo ok > inside.txt && cat inside.txt", 10);
    assert!(r.ok(), "{r:?}");
    assert_eq!(fs::read_to_string(e.sb.work.join("inside.txt")).unwrap().trim(), "ok");
}

#[test]
fn writes_outside_the_copy_are_blocked() {
    let Some(e) = setup(false) else { return };
    let target = e.outside.join("escaped.txt");
    let r = sh(&e, &format!("echo x > '{}'", target.display()), 10);
    assert!(!r.ok());
    assert!(!target.exists(), "wrote outside the copy");
    // The real home folder too.
    let home = capture::fsx::home_dir().unwrap().join(".cv-escape-test");
    let r = sh(&e, &format!("echo x > '{}'", home.display()), 10);
    assert!(!r.ok() && !home.exists(), "wrote to the real home folder");
}

#[test]
fn secrets_are_unreadable() {
    let Some(e) = setup(false) else { return };
    let r = sh(&e, &format!("cat '{}'", e.secret.join("id_rsa").display()), 10);
    assert!(!r.ok() && !r.stdout.contains("PRIVATE KEY"), "{r:?}");
    let ssh = capture::fsx::home_dir().unwrap().join(".ssh");
    if ssh.exists() {
        let r = sh(&e, &format!("ls '{}'", ssh.display()), 10);
        assert!(!r.ok(), "listed ~/.ssh: {}", r.stdout);
    }
}

#[test]
fn external_network_is_blocked_even_with_loopback_on() {
    // With loopback on, the profile must still block outbound traffic: a
    // broad local-ip rule plus Unix sockets let it through in the spike.
    let Some(e) = setup(true) else { return };
    let r = sh(&e, "/usr/bin/curl -s -m 5 -o /dev/null -w '%{http_code}' https://example.com", 15);
    assert!(!r.ok() && !r.stdout.contains("200"), "{r:?}");
    let r = sh(&e, "/usr/bin/curl -s -m 5 -o /dev/null -w '%{http_code}' http://1.1.1.1", 15);
    assert!(!r.ok() && !r.stdout.contains("30") && !r.stdout.contains("200"), "{r:?}");
}

#[test]
fn loopback_works_when_allowed_and_not_otherwise() {
    let script = "const http=require('http');const s=http.createServer((q,r)=>r.end('pong')).listen(0,'127.0.0.1',async()=>{try{const t=await (await fetch('http://127.0.0.1:'+s.address().port)).text();console.log(t)}catch(e){console.log('blocked')}s.close()});s.on('error',()=>{console.log('blocked');process.exit(0)});";
    {
        let Some(on) = setup(true) else { return };
        fs::write(on.sb.work.join("srv.js"), script).unwrap();
        let r = run(&on.sb, &on.sb.work, &["node", "srv.js"], &[], Duration::from_secs(15)).unwrap();
        assert_eq!(r.stdout.trim(), "pong", "{r:?}");
    }
    // Docker can't take the container's own loopback away; outside traffic
    // is blocked either way (previous test).
    if detect_backend() == Backend::Docker {
        return;
    }
    let Some(off) = setup(false) else { return };
    fs::write(off.sb.work.join("srv.js"), script).unwrap();
    let r = run(&off.sb, &off.sb.work, &["node", "srv.js"], &[], Duration::from_secs(15)).unwrap();
    assert_ne!(r.stdout.trim(), "pong", "{r:?}");
}

#[test]
fn unix_sockets_only_inside_the_sandbox_folders() {
    let Some(e) = setup(false) else { return };
    let script = |path: &Path| format!("const s=require('net').createServer().listen('{}',()=>{{console.log('listening');s.close()}});s.on('error',e=>console.log(e.code));", path.display());
    fs::write(e.sb.work.join("inside.js"), script(&e.sb.tmp.join("ipc.sock"))).unwrap();
    let r = run(&e.sb, &e.sb.work, &["node", "inside.js"], &[], Duration::from_secs(10)).unwrap();
    assert_eq!(r.stdout.trim(), "listening", "{r:?}");
    fs::write(e.sb.work.join("outside.js"), script(&e.outside.join("ipc.sock"))).unwrap();
    let r = run(&e.sb, &e.sb.work, &["node", "outside.js"], &[], Duration::from_secs(10)).unwrap();
    assert_ne!(r.stdout.trim(), "listening", "bound a socket outside the sandbox folders");
}

#[test]
fn environment_is_scrubbed() {
    let Some(e) = setup(false) else { return };
    // SAFETY: tests in this file don't read this variable concurrently.
    unsafe { std::env::set_var("TATTLE_TEST_FAKE_TOKEN", "sk-test-should-not-leak") };
    let r = sh(&e, "env", 10);
    assert!(r.ok());
    assert!(!r.stdout.contains("sk-test-should-not-leak"), "token leaked");
    let home = exec::docker::translate(&e.sb.tmp.display().to_string(), &exec::docker::mounts(&e.sb));
    assert!(r.stdout.contains(&format!("HOME={home}")), "{}", r.stdout);
    assert!(!r.stdout.contains("ANTHROPIC") && !r.stdout.contains("GITHUB_TOKEN"));
}

#[test]
fn timeout_kills_the_whole_process_group() {
    let Some(e) = setup(false) else { return };
    let marker = format!("cv-escape-{}", std::process::id());
    // Background children that would outlive their parent.
    let script = format!("(exec -a {marker} sleep 300) & (exec -a {marker} sleep 300) & sleep 300");
    let r = bash(&e, &script, 2);
    assert!(r.timed_out && r.millis < 10_000, "{r:?}");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(survivors(&marker).unwrap_or_default(), "", "children survived the timeout");
}

#[test]
fn a_process_bomb_is_stopped_by_the_timeout() {
    let Some(e) = setup(false) else { return };
    let marker = format!("cv-bomb-{}", std::process::id());
    // Spawns processes until killed (bounded so a failed test can't hang the machine).
    let script = format!("i=0; while [ $i -lt 400 ]; do (exec -a {marker} sleep 300) & i=$((i+1)); done; wait");
    let r = bash(&e, &script, 3);
    assert!(r.timed_out || !r.ok(), "{r:?}");
    std::thread::sleep(Duration::from_millis(800));
    assert_eq!(survivors(&marker).unwrap_or_default(), "", "bomb processes survived");
}

#[test]
fn node_runs_inside() {
    let Some(e) = setup(false) else { return };
    let r = run(&e.sb, &e.sb.work, &["node", "-e", "console.log(1+1)"], &[], Duration::from_secs(10)).unwrap();
    assert_eq!(r.stdout.trim(), "2", "{r:?}");
}

/// Gate 4b.0 part 2: a real project's vitest suite runs in the sandbox,
/// using node_modules from an earlier recording (read-only, outside the
/// copy). Skipped when that folder isn't on this machine.
#[test]
fn vitest_suite_runs_inside() {
    let nm = capture::fsx::home_dir().unwrap().join("Code/cv-sandbox/G2-run1/node_modules");
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/bases/G2");
    if !nm.is_dir() || !base.is_dir() {
        eprintln!("skipped: no G2 node_modules on this machine");
        return;
    }
    let Some(e) = setup(true) else { return };
    copy_dir(&base, &e.sb.work);
    exec::link_node_modules(&nm, &e.sb.work).unwrap();
    let r = run(&e.sb, &e.sb.work, &["node", "node_modules/vitest/vitest.mjs", "run"], &[], Duration::from_secs(120)).unwrap();
    let text = format!("{}{}", r.stdout, r.stderr);
    assert!(r.ok() && text.contains("27 passed"), "{text}");
    // The shared node_modules stayed untouched; the cache went into the copy.
    assert!(e.sb.work.join("node_modules/.vite").is_dir() || !nm.join(".vite/vitest/results.json").exists());
}

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
