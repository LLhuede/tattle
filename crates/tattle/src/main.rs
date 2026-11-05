//! `tattle`: checks AI coding agent output against what was asked. No AI involved.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use capture::export::{BUDGET_BYTES, ExportOptions};
use capture::hook::HookEnv;
use clap::{Parser, Subcommand};

mod config;
mod live;

#[derive(Parser)]
#[command(
    name = "tattle",
    version,
    about = "Catches AI coding agents' hallucinations about their own work. No AI involved."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Record one Claude Code hook event (JSON on stdin). Called by hooks;
    /// always exits 0 and prints nothing.
    Hook {
        /// Event name, if the payload doesn't include one.
        event: Option<String>,
    },
    /// Work with recorded sessions.
    #[command(subcommand)]
    Record(RecordCommand),
    /// Prepare folders for recording test sessions.
    #[command(subcommand)]
    Sandbox(SandboxCommand),
    /// Opt-in: run the project's own tests on a sandboxed copy of a
    /// snapshot, without network (Phase 4b). Prints a JSON report.
    Verify {
        /// Recording folder, e.g. fixtures/recorded/R35/run1.
        recording: PathBuf,
        /// Snapshot commit to verify (default: the final snapshot).
        #[arg(long)]
        commit: Option<String>,
        /// Verify the state at the end of human turn N (1-based) against its
        /// start, instead of the whole session.
        #[arg(long)]
        turn: Option<usize>,
        /// Folder with the original project (its node_modules is tried first).
        #[arg(long)]
        project: Option<PathBuf>,
        /// Folders of other projects whose node_modules may be linked.
        #[arg(long)]
        pool: Vec<PathBuf>,
        /// Seconds before the test run is stopped.
        #[arg(long, default_value_t = 120)]
        timeout: u64,
        /// Save the execution record as <recording>/exec/record.json, where
        /// `tattle check` reads it.
        #[arg(long)]
        write: bool,
        /// Docker: allow downloading Linux dependencies from the npm
        /// registry (install scripts off; the project's code never runs
        /// with network). Without it only the existing pool is used.
        #[arg(long)]
        allow_install: bool,
        /// Docker: the Linux dependency pool (default ~/.cv/linux-deps, or
        /// TATTLE_LINUX_DEPS).
        #[arg(long)]
        linux_deps: Option<PathBuf>,
    },
    /// Run any command inside the execution sandbox (no external network,
    /// writes only in --work and --tmp), the same sandbox as `tattle verify`.
    /// Prints a JSON result.
    SandboxRun {
        #[arg(long)]
        work: PathBuf,
        #[arg(long)]
        tmp: PathBuf,
        /// Allow localhost (test servers, app start).
        #[arg(long)]
        loopback: bool,
        #[arg(long, default_value_t = 600)]
        timeout: u64,
        /// Extra environment variables, KEY=VALUE.
        #[arg(long)]
        env: Vec<String>,
        /// The command and its arguments (after `--`).
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Score the checker against every fixture (synthetic, recorded and
    /// mutated): precision and recall per check, false alarms, determinism.
    Score {
        /// Folder of exported recordings.
        #[arg(long, default_value = "fixtures/recorded")]
        recorded: PathBuf,
        /// Folder of labels (`<prompt>/<run>.json`), kept in git while
        /// recordings aren't.
        #[arg(long, default_value = "corpus/labels")]
        labels: PathBuf,
        /// Also save the full results to harness/results/<date>-<commit>.json.
        #[arg(long)]
        save: bool,
        /// Print the full results as JSON instead of the summary table.
        #[arg(long)]
        json: bool,
        /// Keep the generated fixtures in this folder (for inspection).
        #[arg(long)]
        keep: Option<PathBuf>,
        /// Corpus prompt files, for the intent-extraction score.
        #[arg(long, default_value = "corpus/prompts")]
        corpus: PathBuf,
        /// Base apps the edit prompts start from (for grounding).
        #[arg(long, default_value = "corpus/bases")]
        bases: PathBuf,
        /// Development prompts (used to tune the lexicon), scored separately.
        #[arg(long, default_value = "corpus/dev")]
        dev: PathBuf,
        /// Held-out prompts (never used to tune the lexicon), scored separately.
        #[arg(long, default_value = "corpus/heldout")]
        heldout: PathBuf,
    },
    /// Show the intent facts extracted from a prompt (Phase 5).
    Intent {
        /// The prompt. Give several for a multi-turn conversation.
        #[arg(required = true)]
        turns: Vec<String>,
        /// Project folder to ground targets in (files, names, routes).
        #[arg(long)]
        project: Option<PathBuf>,
        /// Print the full extraction as JSON.
        #[arg(long)]
        json: bool,
    },
    /// What tattle found in this project's latest session, in plain words
    /// (the plugin's /tattle:report command).
    Report {
        /// Project folder (default: $CLAUDE_PROJECT_DIR, else the current folder).
        #[arg(long)]
        project: Option<PathBuf>,
        /// Session id (default: the most recent session).
        #[arg(long)]
        session: Option<String>,
    },
    /// The plugin's settings for this project: mute or unmute a rule, send
    /// alerts to Claude, or show them (`/tattle:mute`, `/tattle:unmute`,
    /// `/tattle:tell-claude`).
    Config {
        /// mute, unmute, tell-claude or show.
        action: String,
        /// The rule id (mute, unmute) or on/off (tell-claude).
        value: Option<String>,
        /// Project folder (default: $CLAUDE_PROJECT_DIR, else the current folder).
        #[arg(long)]
        project: Option<PathBuf>,
    },
    /// Run every check on one exported recording and print the findings.
    Check {
        /// Recording folder, e.g. fixtures/recorded/G1/run1.
        recording: PathBuf,
        /// Print findings as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Generate the synthetic recordings (for inspection).
    Synth {
        /// Output folder; one subfolder per scenario.
        #[arg(long)]
        out: PathBuf,
    },
    /// Inject defects into a recording (for inspection).
    Mutate {
        /// Recording folder to mutate.
        #[arg(long)]
        from: PathBuf,
        /// Output folder; one subfolder per mutation.
        #[arg(long)]
        out: PathBuf,
        /// Only this mutation (default: all).
        #[arg(long)]
        only: Option<String>,
    },
}

#[derive(Subcommand)]
enum RecordCommand {
    /// List recorded sessions in a project.
    List {
        /// Project folder that was recorded.
        #[arg(long, default_value = ".")]
        project: PathBuf,
    },
    /// Copy a recorded session into the test corpus.
    Export {
        /// Project folder that was recorded.
        #[arg(long, default_value = ".")]
        project: PathBuf,
        /// Session id (default: the most recent session).
        #[arg(long)]
        session: Option<String>,
        /// Corpus prompt id, e.g. G1.
        #[arg(long)]
        prompt_id: String,
        /// Run number for this prompt.
        #[arg(long)]
        run: u32,
        /// Output folder, e.g. fixtures/recorded/G1/run1.
        #[arg(long)]
        out: PathBuf,
        /// Folder of corpus prompt files, used to pre-fill labels.json.
        #[arg(long, default_value = "corpus/prompts")]
        corpus: PathBuf,
        /// Export even if the snapshots contain what look like secrets.
        /// Only for synthetic test values.
        #[arg(long)]
        allow_snapshot_secrets: bool,
    },
    /// Rebuild an app folder from a recording's final snapshot
    /// (used to make starting apps in corpus/bases/).
    Restore {
        /// Recording folder, e.g. fixtures/recorded/G1/run1.
        #[arg(long)]
        from: PathBuf,
        /// Folder to create, e.g. corpus/bases/G1.
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Subcommand)]
enum SandboxCommand {
    /// Create a fresh folder with recording hooks set up.
    Init {
        /// Folder to create, e.g. ~/sandboxes/G1-run1.
        dir: PathBuf,
        /// Copy this app in first (for edit prompts).
        #[arg(long)]
        base: Option<PathBuf>,
        /// Reuse a folder that isn't empty.
        #[arg(long)]
        force: bool,
        /// Ask the agent for a manifest of what it did (Phase 6b).
        #[arg(long)]
        manifest: bool,
    },
}

fn main() -> ExitCode {
    // The hook path is handled before argument parsing: a usage error must
    // never reach Claude Code as a non-zero exit (exit 2 blocks tool calls).
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("hook") {
        run_hook(args.get(2).cloned());
        return ExitCode::SUCCESS;
    }

    let cli = Cli::parse();
    let result = match cli.command {
        Command::Hook { event } => {
            run_hook(event);
            Ok(())
        }
        Command::Record(RecordCommand::List { project }) => list(project),
        Command::Record(RecordCommand::Export {
            project,
            session,
            prompt_id,
            run,
            out,
            corpus,
            allow_snapshot_secrets,
        }) => export(ExportOptions {
            project,
            session,
            prompt_id,
            run,
            out,
            corpus: Some(corpus),
            allow_snapshot_secrets,
        }),
        Command::Record(RecordCommand::Restore { from, out }) => {
            capture::restore::restore(&from, &out).map(|chosen| {
                println!("Restored {chosen} into {}", out.display());
            })
        }
        Command::Sandbox(SandboxCommand::Init { dir, base, force, manifest }) => {
            sandbox_init(dir, base, force, manifest)
        }
        Command::SandboxRun { work, tmp, loopback, timeout, env, command } => sandbox_run(work, tmp, loopback, timeout, &env, &command),
        Command::Verify { recording, commit, turn, project, pool, timeout, write, allow_install, linux_deps } => {
            let linux = exec::linuxdeps::Options { root: linux_deps.unwrap_or_else(exec::linuxdeps::default_root), allow_install };
            verify(&recording, commit, turn, project, &pool, timeout, write, linux)
        }
        Command::Score { recorded, labels, save, json, keep, corpus, bases, dev, heldout } => score(recorded, labels, save, json, keep, corpus, bases, dev, heldout),
        Command::Intent { turns, project, json } => intent(turns, project, json),
        Command::Check { recording, json } => check(recording, json),
        Command::Report { project, session } => {
            let project = project
                .or_else(|| std::env::var_os("CLAUDE_PROJECT_DIR").filter(|v| !v.is_empty()).map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("."));
            live::report(&project, session.as_deref()).map(|text| print!("{text}"))
        }
        Command::Config { action, value, project } => {
            let project = project
                .or_else(|| std::env::var_os("CLAUDE_PROJECT_DIR").filter(|v| !v.is_empty()).map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("."));
            // A mistyped rule or setting is a message for the person (the
            // slash command shows stdout), not a failure.
            match config::command(&project, &action, value.as_deref()) {
                Ok(text) => print!("{text}"),
                Err(e) => println!("{e:#}"),
            }
            Ok(())
        }
        Command::Synth { out } => synth(out),
        Command::Mutate { from, out, only } => mutate(from, out, only),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run_hook(event: Option<String>) {
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    let env = HookEnv::from_process();
    // Silence the default panic message; panics are logged instead.
    std::panic::set_hook(Box::new(|_| {}));
    if capture::hook::plugin_skip(&input, &env) {
        return;
    }
    let outcome =
        std::panic::catch_unwind(|| capture::hook::record(event.as_deref(), &input, &env));
    match outcome {
        Ok(Ok(_)) => {
            // The hook prints only in manifest mode (a request for the agent)
            // and in plugin mode at the end of a turn (a line for the person).
            if let Some(text) = capture::hook::manifest_response(&input, &env) {
                print!("{text}");
            } else if env.plugin_mode {
                live::session_start(&input, &env);
                if let Some(text) = live::stop_response(&input, &env) {
                    print!("{text}");
                }
            }
        }
        Ok(Err(e)) => capture::hook::log_error(&env, &format!("{e:#}")),
        Err(panic) => {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "unknown panic".into());
            capture::hook::log_error(&env, &format!("panic: {msg}"));
        }
    }
}

fn list(project: PathBuf) -> Result<()> {
    let sessions = project.join(capture::store::CHECKER_DIR).join("sessions");
    let Ok(entries) = std::fs::read_dir(&sessions) else {
        println!("No recorded sessions in {}", project.display());
        return Ok(());
    };
    let mut rows = Vec::new();
    for entry in entries.flatten() {
        let events = std::fs::read_to_string(entry.path().join("events.jsonl")).unwrap_or_default();
        let lines: Vec<&str> = events.lines().filter(|l| !l.is_empty()).collect();
        let last_event = lines
            .last()
            .and_then(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .and_then(|v| v["event"].as_str().map(str::to_string))
            .unwrap_or_else(|| "-".into());
        rows.push((entry.file_name().to_string_lossy().into_owned(), lines.len(), last_event));
    }
    rows.sort();
    println!("{:<40} {:>7}  last event", "session", "events");
    for (id, n, last) in rows {
        println!("{id:<40} {n:>7}  {last}");
    }
    Ok(())
}

fn export(opts: ExportOptions) -> Result<()> {
    let report = capture::export::export(&opts)?;
    println!("Exported session {} to {}", report.session, report.out.display());
    for (name, bytes) in &report.files {
        println!("  {name:<18} {:>8.1} KB", *bytes as f64 / 1024.0);
    }
    println!(
        "  {:<18} {:>8.1} KB (budget {} KB)",
        "total",
        report.total_bytes as f64 / 1024.0,
        BUDGET_BYTES / 1024
    );
    if report.over_budget {
        println!("WARNING: over the size budget. Check what was captured before committing.");
    }
    if !report.snapshot_secrets.is_empty() {
        println!(
            "Note: {} synthetic secret(s) allowed in snapshots (listed in meta.json).",
            report.snapshot_secrets.len()
        );
    }
    if report.labels_created {
        println!("Next: fill in labels.json in that folder.");
    }
    Ok(())
}

fn sandbox_run(work: PathBuf, tmp: PathBuf, loopback: bool, timeout: u64, env: &[String], command: &[String]) -> Result<()> {
    std::fs::create_dir_all(&tmp)?;
    let sb = exec::Sandbox { work: work.clone(), tmp, deny_read: vec![], loopback, node_modules: None };
    let env: Vec<(&str, &str)> = env.iter().filter_map(|kv| kv.split_once('=')).collect();
    let argv: Vec<&str> = command.iter().map(String::as_str).collect();
    let r = exec::run(&sb, &work, &argv, &env, std::time::Duration::from_secs(timeout))?;
    println!("{}", serde_json::json!({"exit": r.exit, "timed_out": r.timed_out, "millis": r.millis as u64, "stdout": r.stdout, "stderr": r.stderr}));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn verify(recording: &std::path::Path, commit: Option<String>, turn: Option<usize>, project: Option<PathBuf>, pool: &[PathBuf], timeout: u64, write: bool, linux: exec::linuxdeps::Options) -> Result<()> {
    if exec::detect_backend() == exec::Backend::Unavailable {
        println!("{}", serde_json::json!({"verifiable": false, "reason": "no sandbox on this machine (needs macOS, or Docker running)"}));
        return Ok(());
    }
    let rec = checks::Recording::load(recording)?;
    let snaps = rec.snapshots();
    let human: Vec<usize> = snaps
        .iter()
        .enumerate()
        .filter(|(_, s)| s.event == "UserPromptSubmit" && !rec.events[s.index]["payload"]["prompt"].as_str().is_some_and(|p| p.trim_start().starts_with("<task-notification>")))
        .map(|(i, _)| i)
        .collect();
    // Baseline: the first human prompt, or turn N's prompt. Final: the end
    // of the session, or the last snapshot before turn N+1.
    let (baseline, final_commit) = match turn {
        Some(n) => {
            let start = *human.get(n.saturating_sub(1)).context("no such turn")?;
            let end = human.get(n).map_or(snaps.len(), |&i| i).saturating_sub(1);
            (Some(snaps[start].commit.clone()), snaps[end.max(start)].commit.clone())
        }
        None => (
            human.first().map(|&i| snaps[i].commit.clone()),
            match commit {
                Some(c) => c,
                None => rec.final_snapshot().context("recording has no snapshots")?.commit,
            },
        ),
    };
    let sources = exec::verify::DepSources { project: project.into_iter().collect(), pool: pool.to_vec(), linux };
    let record = exec::verify::verify_session(&rec.git().git_dir, baseline.as_deref(), &final_commit, &sources, std::time::Duration::from_secs(timeout))?;
    if write && turn.is_none() {
        let dir = recording.join("exec");
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("record.json"), serde_json::to_string_pretty(&record)? + "\n")?;
    }
    println!("{record}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn score(recorded: PathBuf, labels: PathBuf, save: bool, json: bool, keep: Option<PathBuf>, corpus: PathBuf, bases: PathBuf, dev: PathBuf, heldout: PathBuf) -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let work = match &keep {
        Some(dir) => {
            if dir.exists() && std::fs::read_dir(dir)?.next().is_some() {
                anyhow::bail!("{} is not empty", dir.display());
            }
            dir.clone()
        }
        None => tmp.path().to_path_buf(),
    };
    let collected = harness::score::collect_with_labels(&recorded, labels.is_dir().then_some(labels.as_path()), &work)?;
    let report = harness::score::score(collected, &checks::run);
    let intent = harness::intent_score::score_intent(&corpus, &bases).ok();
    let dev_score = if dev.is_dir() { harness::intent_score::score_intent(&dev, &bases).ok() } else { None };
    // Every corpus/heldout* folder is a separate held-out set.
    let mut held_dirs: Vec<PathBuf> = heldout
        .parent()
        .and_then(|p| std::fs::read_dir(p).ok())
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("heldout")))
                .collect()
        })
        .unwrap_or_default();
    held_dirs.sort();
    let held: Vec<(String, harness::intent_score::IntentScore)> = held_dirs
        .iter()
        .filter_map(|d| Some((d.file_name()?.to_string_lossy().into_owned(), harness::intent_score::score_intent(d, &bases).ok()?)))
        .collect();
    let mut results = report.to_json();
    if let Some(i) = &intent {
        results["intent"] = i.to_json();
        results["prompt_coverage_invariant"] = results["intent"]["coverage_invariant"].clone();
    }
    if let Some(d) = &dev_score {
        results["intent_dev"] = d.to_json();
    }
    for (name, h) in &held {
        results[format!("intent_{name}")] = h.to_json();
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        print!("{}", report.to_text());
        if let Some(i) = &intent {
            print!("{}", i.to_text());
        }
        if let Some(d) = &dev_score {
            println!("\nDevelopment prompts (used to tune the lexicon, so optimistic):");
            print!("{}", d.to_text());
        }
        for (name, h) in &held {
            println!("\nHeld-out prompts, corpus/{name} (never used to tune the lexicon):");
            print!("{}", h.to_text());
        }
    }
    if save {
        let path = save_results(&results)?;
        eprintln!("Saved results to {}", path.display());
    }
    if let Some(dir) = keep {
        eprintln!("Generated fixtures kept in {}", dir.display());
    }
    Ok(())
}

/// Save to harness/results/<YYYY-MM-DD>-<commit>[-dirty].json.
fn save_results(results: &serde_json::Value) -> Result<PathBuf> {
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "nocommit".into());
    let dirty = git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let name = format!("{}-{commit}{}.json", today(), if dirty { "-dirty" } else { "" });
    let dir = PathBuf::from("harness/results");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_string_pretty(results)? + "\n")?;
    Ok(path)
}

/// Today's date (UTC) as YYYY-MM-DD, without a date library.
fn today() -> String {
    let days = (capture::hook::now_ms() / 86_400_000) as i64;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn check(recording: PathBuf, json: bool) -> Result<()> {
    let rec = checks::Recording::load(&recording)?;
    let report = checks::run(&rec);
    if json {
        let findings: Vec<serde_json::Value> = report.findings.iter().map(checks::Finding::to_json).collect();
        let out = serde_json::json!({ "findings": findings, "intent": report.intent });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    if report.findings.is_empty() {
        println!("No findings.");
    }
    for f in &report.findings {
        let file = f.file.as_deref().unwrap_or("-");
        let seq = f.seq.map_or("-".to_string(), |s| s.to_string());
        println!("{:<30} {:<28} seq {:<5} {}", f.check, file, seq, f.message);
    }
    let facts: Vec<&serde_json::Value> = report.intent.iter().filter(|r| r.get("fact").is_some()).collect();
    if !facts.is_empty() {
        println!("\nWhat you asked:");
        for r in facts {
            let icon = match (r["verdict"].as_str(), r["precise"].as_bool()) {
                (Some("verified"), _) => "✅",
                (Some("violated"), _) => "⚠️ ",
                (_, Some(true)) => "❔",
                _ => "ℹ️ ",
            };
            println!("  {icon} \"{}\" ({}): {}", r["fact"]["source"]["text"].as_str().unwrap_or(""), r["fact"]["kind"].as_str().unwrap_or(""), r["evidence"].as_str().unwrap_or(""));
        }
    }
    if let Some(m) = report.intent.iter().find_map(|r| r.get("manifest")).and_then(|m| m.as_array()) {
        let backed = m.iter().filter(|q| q["backed"] == true).count();
        println!("\nAgent's manifest: {} requirement(s), {backed} backed by the code", m.len());
        for q in m {
            let icon = if q["backed"] == true { "✅" } else if q["unsupported"].as_array().is_some_and(|u| !u.is_empty()) { "⚠️ " } else { "ℹ️ " };
            println!("  {icon} [{}] {}", q["status"].as_str().unwrap_or(""), q["request"].as_str().unwrap_or(""));
        }
    }
    Ok(())
}

fn intent(turns: Vec<String>, project: Option<PathBuf>, json: bool) -> Result<()> {
    let index = match &project {
        Some(dir) => checks::intent::RepoIndex::build(&harness::intent_score::read_project(dir)),
        None => checks::intent::RepoIndex::default(),
    };
    let refs: Vec<&str> = turns.iter().map(String::as_str).collect();
    let e = checks::intent::extract(&refs, &index);
    if json {
        println!("{}", serde_json::to_string_pretty(&e.to_json())?);
        return Ok(());
    }
    if e.facts.is_empty() {
        println!("No checkable facts.");
    }
    for f in &e.facts {
        let value = f.value.as_ref().map(|v| v.to_string()).unwrap_or_default();
        let unit = f.unit.as_deref().unwrap_or("");
        let targets: Vec<String> = f
            .targets
            .iter()
            .map(|t| {
                let where_ = if t.grounded.is_empty() { String::new() } else { format!(" = {}", t.grounded.join(", ")) };
                format!("{} ({}{where_})", t.text, t.confidence.as_str())
            })
            .collect();
        println!("{:<26} {:<8} {value} {unit} {}  ← \"{}\"", f.kind, f.polarity.as_str(), targets.join("; "), f.source.text);
    }
    for r in &e.replaced {
        println!("(replaced by a later turn) {} {}  ← \"{}\"", r.kind, r.polarity.as_str(), r.source.text);
    }
    let unchecked: Vec<&str> = e.residual.iter().map(|r| r.text.as_str()).collect();
    if !unchecked.is_empty() {
        println!("\nCouldn't check: {}", unchecked.join(" · "));
    }
    let c = &e.coverage;
    println!(
        "\nEvery word accounted for: {} (used by facts {} · couldn't check {} · filler {} of {} characters)",
        if c.holds() { "yes" } else { "NO" },
        c.consumed_chars, c.residual_chars, c.filler_chars, c.total_chars
    );
    Ok(())
}

fn synth(out: PathBuf) -> Result<()> {
    for s in harness::synth::scenarios() {
        let dir = out.join(s.name);
        harness::synth::generate(&s, &dir)?;
        println!("{}", dir.display());
    }
    Ok(())
}

fn mutate(from: PathBuf, out: PathBuf, only: Option<String>) -> Result<()> {
    let names: Vec<&str> = match &only {
        Some(name) => vec![name.as_str()],
        None => harness::mutate::MUTATIONS.iter().map(|m| m.name).collect(),
    };
    for name in names {
        match harness::mutate::apply(&from, name, &out.join(name))? {
            Some(a) => println!("{name:<22} {}", a.detail),
            None => println!("{name:<22} (doesn't apply to this recording)"),
        }
    }
    Ok(())
}

fn sandbox_init(dir: PathBuf, base: Option<PathBuf>, force: bool, manifest: bool) -> Result<()> {
    let exe = capture::fsx::canonical(&std::env::current_exe()?)?;
    let dir = capture::sandbox::init(&dir, base.as_deref(), force, &exe, manifest)?;
    println!("Sandbox ready: {}", dir.display());
    println!("Recording hooks call: {}", exe.display());
    println!();
    println!("Next:");
    println!("  1. Open that folder in Claude Code (a new session).");
    println!("  2. Paste the prompt exactly as written in the corpus file.");
    println!("  3. When the agent finishes, export the recording:");
    println!(
        "     tattle record export --project {} --prompt-id <ID> --run <N> --out fixtures/recorded/<ID>/run<N>",
        capture::sandbox::shell_quote(&dir.to_string_lossy())
    );
    Ok(())
}
