//! The checker (Phases 2–6 of docs/BUILD_PLAN.md).
//!
//! Input: a [`Recording`] (an exported session). Output: a [`Report`] of
//! findings. Deterministic and AI-free.
//!
//! Implemented so far (Phase 2–3, behaviour):
//! - `capture.sequence_gap`, `capture.incomplete`
//! - `events.changed_by_command`, `events.unexplained_change`
//! - `tests.edited_after_failure`, `claims.tests_pass_false`
//! - `events.edited_without_reading`, `tests.not_rerun_after_edit`
//! - `commands.risky`, `events.edit_outside_project`
//!
//! Phase 4 (code, before vs after each turn):
//! - `tests.removed`, `tests.assertion_weakened`, `tests.skipped`
//! - `code.stub`, `code.swallowed_error`, `deps.added`
//! - `secrets.hardcoded`, `scope.unrelated_file`

pub mod asks;
pub mod behaviour;
pub mod catalog;
pub mod code;
pub mod events;
pub mod exec_findings;
pub mod fix_claims;
pub mod imports;
pub mod intent;
pub mod intent_checks;
pub mod manifest;
pub mod paths;
pub mod plain;
pub mod recording;
pub mod report;
pub mod transcript;
pub mod ts;

use std::collections::{BTreeMap, BTreeSet};

pub use recording::Recording;
pub use report::{Finding, Report};

use behaviour::Turn;

/// Run every check on a recording.
pub fn run(recording: &Recording) -> Report {
    let events = events::normalise(&recording.events, &recording.transcript);
    let transcript_results: BTreeSet<String> = events::transcript_results(&recording.transcript).into_keys().collect();
    let (turns, unclosed, code_turns) = turns(recording, &events);

    let mut findings = Vec::new();
    findings.extend(behaviour::sequence_gaps(&events));
    findings.extend(behaviour::capture_problems(&events, unclosed));
    findings.extend(behaviour::unfinished_tool_calls(&events, &transcript_results));
    let bounds = session_bounds(recording);
    findings.extend(
        behaviour::changes_outside_edit_tools(&turns)
            .into_iter()
            .filter(|f| !f.file.as_deref().is_some_and(|p| bounds.transient(p))),
    );
    findings.extend(behaviour::tests_edited_after_failure(&turns));
    findings.extend(behaviour::false_tests_pass_claim(&events, &recording.transcript));
    findings.extend(behaviour::unverified_tests_pass_claim(&events, &recording.transcript));
    // Findings that belong to one request, keyed without their `seq`: the
    // seq of that request's prompt is filled in after de-duplication, so the
    // set of findings doesn't change.
    let mut at_prompt: BTreeMap<Finding, u64> = BTreeMap::new();
    let mut note = |seq: u64, f: Finding, out: &mut Vec<Finding>| {
        if f.seq.is_none() {
            at_prompt.entry(f.clone()).or_insert(seq);
        }
        out.push(f);
    };
    for (seq, f) in prompt_turn_checks(recording, &events) {
        note(seq, f, &mut findings);
    }
    findings.extend(fix_claims::fix_unreproduced(&recording.events, &events));
    findings.extend(fix_claims::flaky_unobserved(&recording.events, &events));
    findings.extend(fix_claims::tests_pass_other_command(&recording.events, &events));
    findings.extend(exec_findings::findings(recording, &events));
    findings.extend(behaviour::tests_not_rerun_after_edit(&turns));
    findings.extend(behaviour::risky_commands(&events));
    findings.extend(behaviour::edits_outside_project(&events));
    if let Some(start) = recording.baseline()
        && let Ok(files) = recording.list_files(&start.commit)
    {
        let files: BTreeSet<String> = files.into_iter().collect();
        findings.extend(behaviour::edited_without_reading(&events, &files));
    }
    for t in &code_turns {
        let project = ProjectAt { recording, commit: &t.end_commit, test_files: &t.test_files, session: &bounds };
        for f in code::check_turn(&t.changes, &project) {
            note(t.prompt_seq, f, &mut findings);
        }
    }
    let (intent, manifest) = intent_results(recording, &events);
    let prompt_seqs: Vec<u64> = human_prompt_seqs(recording).into_iter().map(|(s, _)| s).collect();
    for (turn, f) in intent_checks::findings_by_turn(&intent) {
        match prompt_seqs.get(turn) {
            Some(&seq) => note(seq, f, &mut findings),
            None => findings.push(f),
        }
    }
    findings.extend(manifest.findings.iter().cloned());
    findings.sort();
    findings.dedup();
    // Claims and the manifest judge the latest request's closing message.
    let last_prompt = prompt_seqs.last().copied();
    for f in findings.iter_mut().filter(|f| f.seq.is_none()) {
        f.seq = at_prompt.get(f).copied().or_else(|| (f.check.starts_with("claims.") || f.check.starts_with("manifest.")).then_some(last_prompt).flatten());
    }
    let mut intent_json: Vec<serde_json::Value> = intent.iter().map(intent_checks::FactResult::to_json).collect();
    if manifest.present {
        intent_json.push(serde_json::json!({ "manifest": manifest.requirements.iter().map(manifest::Requirement::to_json).collect::<Vec<_>>() }));
    }
    Report { findings, intent: intent_json }
}

/// One turn's changed files with contents, the commit at its end, and the
/// contents of every test file at the end.
struct CodeTurn {
    /// Seq of the prompt that started the turn.
    prompt_seq: u64,
    changes: Vec<code::FileChange>,
    end_commit: String,
    test_files: BTreeMap<String, String>,
}

/// The human prompts of a session, oldest first.
pub fn human_prompts(recording: &Recording) -> Vec<String> {
    human_prompt_seqs(recording).into_iter().map(|(_, p)| p).collect()
}

/// The human prompts with their event seq, oldest first. Request N in the
/// report is the Nth of these.
pub fn human_prompt_seqs(recording: &Recording) -> Vec<(u64, String)> {
    recording
        .events
        .iter()
        .filter(|e| e["event"] == "UserPromptSubmit")
        .filter_map(|e| Some((e["seq"].as_u64().unwrap_or(0), e["payload"]["prompt"].as_str()?)))
        .filter(|(_, p)| !p.trim_start().starts_with("<task-notification>"))
        .map(|(s, p)| (s, p.to_string()))
        .collect()
}

/// Phase 6a/6b: the session's intent facts and whether the code bears
/// them out, plus the verified manifest (if any).
fn intent_results(recording: &Recording, events: &[events::Event]) -> (Vec<intent_checks::FactResult>, manifest::Verification) {
    let none = || (Vec::new(), manifest::Verification::default());
    let prompts = human_prompts(recording);
    if prompts.is_empty() {
        return none();
    }
    let snaps = recording.snapshots();
    let Some(start) = snaps.iter().find(|s| is_human_prompt(recording, s)) else { return none() };
    let Some(end) = recording.final_snapshot() else { return none() };
    let (Ok(changes), Ok(before), Ok(after)) = (
        recording.changed_files(&start.commit, &end.commit),
        recording.list_files(&start.commit),
        recording.list_files(&end.commit),
    ) else {
        return none();
    };
    let text_file = |p: &String| !paths::is_lockfile(p);
    let before_files = recording.read_files(&start.commit, &before.into_iter().filter(text_file).collect::<Vec<_>>());
    let after_files = recording.read_files(&end.commit, &after.into_iter().filter(text_file).collect::<Vec<_>>());
    let pairs = |m: &BTreeMap<String, String>| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>();
    let cache_file = recording.cache_dir.as_ref().map(|d| d.join("names-cache.json"));
    let mut names = cache_file.as_deref().map(intent::NameCache::load).unwrap_or_default();
    let index_before = intent::RepoIndex::build_cached(&pairs(&before_files), &mut names);
    let index_after = intent::RepoIndex::build_cached(&pairs(&after_files), &mut names);
    if let Some(f) = &cache_file {
        names.save(f);
    }
    let file_changes: Vec<code::FileChange> = changes
        .iter()
        .map(|c| code::FileChange {
            path: c.path.clone(),
            status: c.status,
            before: before_files.get(&c.path).cloned(),
            after: after_files.get(&c.path).cloned(),
        })
        .collect();
    let last_test_run = events
        .iter()
        .filter(|e| e.index > start.index)
        .rev()
        .find_map(|e| e.tool.as_ref().filter(|t| t.post && t.is_test_run()).map(|t| t.outcome));
    let refs: Vec<&str> = prompts.iter().map(String::as_str).collect();
    let facts = intent::extract(&refs, &index_before).facts;
    let read_only_turns: BTreeSet<usize> = prompts.iter().enumerate().filter(|(_, p)| asks::read_only(p)).map(|(i, _)| i).collect();
    let added_deps_by_turn = deps_added_since_each_turn(recording, &snaps, &end.commit);
    let evidence = intent_checks::SessionEvidence {
        read_only_turns,
        prompts: prompts.clone(),
        added_deps_by_turn,
        changes: &file_changes,
        index_before: &index_before,
        index_after: &index_after,
        code_after: &after_files,
        last_test_run,
    };
    let mut results = intent_checks::evaluate(&facts, &evidence);

    // Manifest: verify its claims, then let backed requirements confirm
    // prompt facts the code checks alone couldn't.
    // The latest manifest describes the latest turn (real session H37: turn
    // 2 undid turn 1), so check it against that turn's changes.
    let requested = recording.events.iter().any(|e| e["manifest_requested"] == true);
    let last_start = snaps.iter().rev().find(|s| is_human_prompt(recording, s)).unwrap_or(start);
    let turn_changes: Vec<code::FileChange> = match recording.changed_files(&last_start.commit, &end.commit) {
        Ok(cs) => {
            let paths: Vec<String> = cs.iter().map(|c| c.path.clone()).collect();
            let before = recording.read_files(&last_start.commit, &paths);
            cs.into_iter()
                .map(|c| code::FileChange {
                    before: before.get(&c.path).cloned(),
                    after: after_files.get(&c.path).cloned(),
                    path: c.path,
                    status: c.status,
                })
                .collect()
        }
        Err(_) => file_changes.clone(),
    };
    // A last turn that only answered a question describes earlier work
    // (real session R18: "did you also add tests?").
    let mut turn_changes = if turn_changes.iter().all(|c| paths::is_lockfile(&c.path)) { file_changes.clone() } else { turn_changes };
    // A build's final manifest describes the whole session: a file changed
    // in an earlier turn still counts (big-test builds B3–B5).
    for c in &file_changes {
        if !turn_changes.iter().any(|t| t.path == c.path) {
            turn_changes.push(c.clone());
        }
    }
    let read_only_session = file_changes.iter().all(|c| paths::is_lockfile(&c.path) || c.path.starts_with(".checker/"));
    let added = intent_checks::added_dependencies(&turn_changes);
    let mev = manifest::Evidence { changes: &turn_changes, index_after: &index_after, code_after: &after_files, added_deps: &added, last_test_run, read_only_session };
    let verification = manifest::verify(recording.manifests.last(), requested, &mev);
    for r in results.iter_mut().filter(|r| r.verdict == intent_checks::Verdict::Unknown && !r.precise) {
        if let Some(req) = verification.requirements.iter().find(|q| q.backed() && manifest::covers(q, &r.fact)) {
            r.verdict = intent_checks::Verdict::Verified;
            r.evidence = format!("manifest: \"{}\" ({})", req.request, req.supported.join(", "));
        }
    }
    (results, verification)
}

/// Per-turn prompt checks: `intent.unrequested_change` and
/// `intent.phantom_reference` (recall study). Each human prompt is judged
/// against the project as it was when its turn started and the files its
/// turn changed.
fn prompt_turn_checks(recording: &Recording, events: &[events::Event]) -> Vec<(u64, Finding)> {
    let snaps = recording.snapshots();
    let agent_text = assistant_text(&recording.transcript, events);
    let mut out = Vec::new();
    for (i, start) in snaps.iter().enumerate() {
        if !is_human_prompt(recording, start) {
            continue;
        }
        let Some(prompt) = recording.events[start.index]["payload"]["prompt"].as_str() else { continue };
        let next_human = snaps[i + 1..].iter().position(|s| is_human_prompt(recording, s)).map_or(snaps.len(), |p| i + 1 + p);
        if let Some(end) = snaps[i + 1..next_human].last()
            && let Ok(changes) = recording.changed_files(&start.commit, &end.commit)
        {
            let changed: Vec<String> = changes.into_iter().map(|c| c.path).collect();
            out.extend(asks::unrequested_changes(prompt, &changed).into_iter().map(|f| (start.seq, f)));
        }
        if asks::references(prompt).is_empty() {
            continue;
        }
        let Ok(files) = recording.list_files(&start.commit) else { continue };
        let code_paths: Vec<String> = files.iter().filter(|f| asks::is_code_file(f) && !paths::is_lockfile(f) && !f.ends_with("package.json")).cloned().collect();
        // A from-scratch build: everything the prompt names is to be made.
        if code_paths.is_empty() {
            continue;
        }
        let code = recording.read_files(&start.commit, &code_paths);
        let files: BTreeSet<String> = files.into_iter().collect();
        let tests = asks::test_names(&code);
        let before = asks::ProjectBefore { files: &files, code: &code, test_names: &tests };
        // What the agent defined during the turn (a name it was asked to add).
        let end_code = snaps[i + 1..next_human].last().and_then(|end| {
            let paths: Vec<String> = recording.list_files(&end.commit).ok()?.into_iter().filter(|f| asks::is_code_file(f)).collect();
            Some(recording.read_files(&end.commit, &paths))
        }).unwrap_or_default();
        out.extend(asks::phantom_references_after(prompt, &before, &end_code, &agent_text).into_iter().map(|f| (start.seq, f)));
    }
    out
}

/// What the agent reported: its closing message of every turn (`Stop`
/// events), or the transcript's last assistant text. Mid-session remarks
/// don't count: real session R01 (run 2) noted "applyDiscount doesn't exist"
/// while working, then reported "fixed the 15% discount rounding issue".
fn assistant_text(transcript: &[serde_json::Value], events: &[events::Event]) -> String {
    let stops: Vec<String> = events.iter().filter(|e| e.name == "Stop").filter_map(|e| e.message.clone()).collect();
    if stops.is_empty() { behaviour::final_assistant_text(transcript).unwrap_or_default() } else { stops.join("\n") }
}

/// For each human turn: dependencies in package.json files at the end of the
/// session that weren't there when the turn started (files that existed
/// then; a created package.json is judged by the session-level rule).
fn deps_added_since_each_turn(recording: &Recording, snaps: &[recording::Snapshot], end_commit: &str) -> Vec<Vec<String>> {
    let manifests = |commit: &str| -> BTreeMap<String, BTreeSet<String>> {
        let files: Vec<String> = recording
            .list_files(commit)
            .unwrap_or_default()
            .into_iter()
            .filter(|f| f.ends_with("package.json") && !f.contains("node_modules/"))
            .collect();
        recording
            .read_files(commit, &files)
            .into_iter()
            .filter_map(|(p, t)| Some((p, code::deps(&serde_json::from_str(&t).ok()?, code::DEP_FIELDS))))
            .collect()
    };
    let at_end = manifests(end_commit);
    snaps
        .iter()
        .filter(|s| is_human_prompt(recording, s))
        .map(|start| {
            let before = manifests(&start.commit);
            before
                .iter()
                .flat_map(|(path, had)| at_end.get(path).map(|now| now.difference(had).cloned().collect::<Vec<_>>()).unwrap_or_default())
                .collect()
        })
        .collect()
}

/// A prompt typed by the human. Claude Code also delivers background-task
/// notices as `UserPromptSubmit` (`<task-notification>…`); those continue
/// the agent's work rather than starting a new turn.
fn is_human_prompt(recording: &Recording, snap: &recording::Snapshot) -> bool {
    snap.event == "UserPromptSubmit"
        && !recording.events[snap.index]["payload"]["prompt"]
            .as_str()
            .is_some_and(|p| p.trim_start().starts_with("<task-notification>"))
}

/// Split the session into agent turns: from each human prompt's snapshot to
/// the last snapshot before the next human prompt. Also returns how many
/// prompts have no closing snapshot (or whose snapshots couldn't be read),
/// and each turn's file contents for the code checks.
fn turns(recording: &Recording, events: &[events::Event]) -> (Vec<Turn>, usize, Vec<CodeTurn>) {
    let snaps = recording.snapshots();
    let mut out = Vec::new();
    let mut code_turns = Vec::new();
    let mut unclosed = 0;
    for (i, start) in snaps.iter().enumerate() {
        if !is_human_prompt(recording, start) {
            continue;
        }
        let next_human = snaps[i + 1..]
            .iter()
            .position(|s| is_human_prompt(recording, s))
            .map_or(snaps.len(), |p| i + 1 + p);
        let Some(end) = snaps[i + 1..next_human].last() else {
            unclosed += 1;
            continue;
        };
        let (Ok(changes), Ok(before), Ok(after)) = (
            recording.changed_files(&start.commit, &end.commit),
            recording.list_files(&start.commit),
            recording.list_files(&end.commit),
        ) else {
            unclosed += 1;
            continue;
        };
        let after: BTreeSet<String> = after.into_iter().collect();
        // Read everything needed in one batch per snapshot.
        let changed: Vec<String> = changes.iter().map(|c| c.path.clone()).collect();
        let mut wanted_end: Vec<String> = after.iter().filter(|f| paths::is_test_file(f)).cloned().collect();
        wanted_end.extend(changed.iter().cloned());
        let mut end_files = recording.read_files(&end.commit, &wanted_end);
        let start_files = recording.read_files(&start.commit, &changed);
        // Imported files are needed for coverage; read the few that are missing.
        let covered = imports::covered_files(&after, |p| {
            end_files.get(p).cloned().or_else(|| recording.read_file(&end.commit, p))
        });
        let file_changes = changes
            .iter()
            .map(|c| code::FileChange {
                path: c.path.clone(),
                status: c.status,
                before: start_files.get(&c.path).cloned(),
                after: end_files.get(&c.path).cloned(),
            })
            .collect();
        end_files.retain(|p, _| paths::is_test_file(p));
        code_turns.push(CodeTurn { prompt_seq: start.seq, changes: file_changes, end_commit: end.commit.clone(), test_files: end_files });
        out.push(Turn {
            files_before: before.into_iter().collect::<BTreeSet<_>>(),
            changes,
            events: events
                .iter()
                .filter(|e| e.index > start.index && e.index < end.index)
                .cloned()
                .collect(),
            covered,
        });
    }
    (out, unclosed, code_turns)
}

/// The project as it was at one snapshot, for the code checks' lookups.
struct ProjectAt<'a> {
    recording: &'a Recording,
    commit: &'a str,
    test_files: &'a BTreeMap<String, String>,
    session: &'a SessionBounds,
}

/// What the session started and ended with, so that changes made and then
/// undone within the session aren't reported.
#[derive(Default)]
struct SessionBounds {
    start_tests: Option<BTreeSet<String>>,
    end_deps: Option<BTreeSet<String>>,
    /// Files at the start and end of the session.
    start_files: Option<BTreeSet<String>>,
    end_files: Option<BTreeSet<String>>,
}

impl SessionBounds {
    /// A file that didn't exist when the session started and doesn't exist
    /// at the end was created and removed within the session (real session
    /// H37: "scrap redux" deleted the store files turn 1 had made).
    fn transient(&self, path: &str) -> bool {
        match (&self.start_files, &self.end_files) {
            (Some(s), Some(e)) => !s.contains(path) && !e.contains(path),
            _ => false,
        }
    }
}

fn session_bounds(recording: &Recording) -> SessionBounds {
    let snaps = recording.snapshots();
    let (Some(start), Some(end)) = (snaps.iter().find(|s| is_human_prompt(recording, s)), recording.final_snapshot()) else {
        return SessionBounds::default();
    };
    let start_files = recording.list_files(&start.commit).unwrap_or_default();
    let start_set: BTreeSet<String> = start_files.iter().cloned().collect();
    let tests: Vec<String> = start_files.into_iter().filter(|f| paths::is_test_file(f)).collect();
    let start_tests = recording
        .read_files(&start.commit, &tests)
        .values()
        .flat_map(|src| ts::facts(src).tests.into_iter().map(|t| t.name))
        .collect();
    let end_files = recording.list_files(&end.commit).unwrap_or_default();
    let end_set: BTreeSet<String> = end_files.iter().cloned().collect();
    let packages: Vec<String> = end_files.into_iter().filter(|f| f.ends_with("package.json") && !f.contains("node_modules/")).collect();
    let end_deps = recording
        .read_files(&end.commit, &packages)
        .values()
        .filter_map(|t| serde_json::from_str::<serde_json::Value>(t).ok())
        .flat_map(|v| code::deps(&v, code::DEP_FIELDS))
        .collect();
    SessionBounds { start_tests: Some(start_tests), end_deps: Some(end_deps), start_files: Some(start_set), end_files: Some(end_set) }
}

impl code::Project for ProjectAt<'_> {
    fn test_names(&self) -> BTreeSet<String> {
        self.test_files
            .values()
            .flat_map(|src| ts::facts(src).tests.into_iter().map(|t| t.name))
            .collect()
    }

    fn word_used_elsewhere(&self, word: &str, except: &str) -> bool {
        self.recording
            .grep_code(self.commit, &["-w", "-F", "-e", word])
            .iter()
            .any(|f| f != except)
    }

    fn session_start_tests(&self) -> Option<BTreeSet<String>> {
        self.session.start_tests.clone()
    }

    fn session_end_deps(&self) -> Option<BTreeSet<String>> {
        self.session.end_deps.clone()
    }

    fn package_imported(&self, name: &str) -> bool {
        let single = format!("'{name}");
        let double = format!("\"{name}");
        !self.recording.grep_code(self.commit, &["-F", "-e", &single, "-e", &double]).is_empty()
    }
}
