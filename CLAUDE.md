# tattle

A local, rule-based checker that catches an AI coding agent's hallucinations: it compares what the agent was asked to do, what it did and what it claimed. It reports findings a human or an agent can act on. Instructions for anyone, human or agent, working on this repo.

## Key documents
- `docs/KNOWLEDGE.md`: **everything learned so far** (how it works, Claude Code facts, what works, gaps, lessons). Read this first.
- `docs/BUILD_PLAN.md`: phases, gates and folder layout (**read first**)
- `docs/TEST_PROMPTS.md`: test corpus prompts, expected facts, traps
- `docs/DESIGN_NOTES.md`: full design reasoning
- `docs/DECISIONS.md`: decision log (newest first)
- `docs/RECORDING.md`: how to record and label test sessions
- `docs/RESULTS.md`: **all results so far: accuracy and gaps** (read before changing priorities)
- `README.md`: the public README. Release steps are kept privately by the maintainer.
- `docs/WINDOWS.md`: setting up and running on Windows (the development machine since 2026-10-29); what works there
- `docs/PHASE_4B_PLAN.md`: the opt-in execution layer: 4b.0–4b.3 built (sandbox, offline dependencies, test/build probes, app start); next steps in `docs/IMPROVEMENT_PLAN.md`
- `bigtests/README.md`: from-scratch build suites A–D (specs, turn scripts, hidden acceptance tests, runners); plan D ran ($228; results in `docs/RESULTS.md` §3g)
- `corpus/prompts/<ID>.json`: machine-readable test prompts (source of truth)

## Commands
```bash
cargo build --release          # builds target/release/tattle
cargo test                     # all tests (unit, end-to-end, corpus checks)
cargo clippy --all-targets     # lint; keep it warning-free
target/release/tattle score        # score the checker (add --save after committing)
```
Run `tattle score` after any checker change (`cargo test` also enforces it when recordings are on disk). `--save` writes `harness/results/`, which is local only. **Never commit recordings or results** (user decision, 2026-10-26). Recordings live in `fixtures/recorded/` and big-test runs in `artifacts/`, both gitignored. What's in git is the reproducible suite: prompts (`corpus/`), bases (`corpus/bases/`), labels (`corpus/labels/`) and big-test definitions (`bigtests/`). Oracle output and reviews are local too.

## Code layout
- `crates/tattle`: the `tattle` binary (thin CLI). `tattle hook` is handled before argument parsing and must always exit 0. It prints only in manifest mode and, in plugin mode, the end-of-turn line for the person (`live.rs`).
- `plugin/`: the Claude Code plugin (hooks, `/tattle:report`, launcher `scripts/tattle`). Binaries go in `plugin/dist/` (gitignored); users install from the `plugin-dist` branch (`scripts/release_plugin.sh`). `crates/checks/src/plain.rs` turns findings into plain sentences.
- This repo records its own development: `.claude/settings.json` runs the plugin launcher from a frozen copy. Refresh it with `scripts/dev_plugin.sh` after a checker change; `/tattle-report` shows the session.
- `crates/capture`: recording: hooks, redaction, placeholders, trimming, shadow-git snapshots, export, restore, sandbox setup
- `crates/checks`: the checker. `catalog.rs` lists finding ids (a contract: add, don't rename). `events.rs` is the Claude Code adapter (all agent-specific parsing lives here). `behaviour.rs` has the Phase 2–3 checks; `code.rs` the Phase 4 checks; `ts.rs` extracts code facts with tree-sitter; `imports.rs` finds what tests import. Checks work on plain data so they can be unit-tested without git.
- `crates/harness`: synthetic recordings (`synth`), defect injection (`mutate`), scoring (`score`), intent scoring (`intent_score`). The checker must never read `expected.json`.
- `crates/checks/src/intent.rs`: prompt → intent facts (Phase 5, lexicon v2)
- `crates/exec`: opt-in sandboxed execution (Phase 4b). `sandbox-exec` profile, runner, escape tests. Never used by `tattle check` or the hooks
- `crates/checks/src/asks.rs`: prompt-level checks without the lexicon (read-only requests, phantom references, unrequested changes)
- `corpus/recall/`, `corpus/recall2/`: recall-study trap prompts (designed to make agents claim false things); `corpus/recall2/` was written by an agent that never saw the checker
- `corpus/dev/`: prompts the lexicon was tuned on · `corpus/heldout/`: prompts **never** used for tuning. Don't change the lexicon based on held-out failures; see `docs/DECISIONS.md` → "Held-out protocol".
- `corpus/prompts/`: test prompts · `corpus/bases/`: frozen starting apps · `fixtures/recorded/`: exported recordings · `harness/results/`: saved scores

## Lessons that must not be relearned (details in docs/KNOWLEDGE.md §6)
- A score on data you built against is optimistic: always measure on a held-out set or a fresh batch of real sessions. Record a batch's numbers before fixing what it exposed.
- Synthetic mutations scoring 1.00 says little about real sessions; real sessions expose situations nobody synthesises.
- Claude Code: failures fire `PostToolUseFailure`; Bash has no exit code; task notices arrive as `UserPromptSubmit`; headless denials fire no hook.
- Never rebuild `target/release/tattle` while sessions record against it (the runner uses a frozen copy).
- Keep the project outside ~/Documents (macOS blocks the Claude Code helper there).

## Hard rules for anyone working here (human or agent)
1. **No AI in the checker.** It never calls a model, agent or network service. Analysis is static by default. Running the user's code is only allowed in the sandboxed, opt-in execution layer (Phase 4b, see `docs/DECISIONS.md`).
2. **Local by design.** The checker never calls the network and sends nothing anywhere. Recordings (`fixtures/recorded/`), big-test runs (`artifacts/`), results, the dependency pool and third-party tools are never committed. On this project's behalf:
   - Don't star, fork, watch, open issues or PRs on, comment on, or contact other projects or their authors unless the maintainer asks.
   - Read-only access to public pages is fine (anonymous, not signed in): research, and downloading a tool's source once to study.
   - Third-party tools run **only inside the sandbox with network off**, after a telemetry check.
   - Don't publish anything about other projects on this project's behalf unless the maintainer decides to.
   - The public repo is `LLhuede/tattle`; the development history is kept privately.
3. **Test first.** No checker logic gets merged without fixtures and a `score` run showing its precision and recall.
4. **Deterministic.** Same input + same rule versions must give byte-identical output.
5. **Never store secrets.** Redact before writing; `.checker/` stays gitignored.
6. **Document as you go:**
   - Update the **Progress log** below at the end of each work session
   - Add an entry to `docs/DECISIONS.md` for every design decision
   - Keep `docs/BUILD_PLAN.md` in line with reality (mark phases done, record changes)
7. **Commit the code, docs and test suite** with clear messages; docs are committed alongside the code they describe. Don't commit results (scores, oracle output, reviews, scratch analysis): they're gitignored. Keep git lean.
   - Commits are authored by the person contributing (their own git identity), never by the agent. Agents keep their `Co-Authored-By` trailer in the message (user decisions, 2026-10-29 and 2026-11-04).

## Current status
- **Built:** Phases 0–6 (recorder, behaviour and code checks, intent, manifest), 4b (opt-in sandboxed execution: macOS sandbox or Docker), and the Claude Code plugin (end-of-turn alerts, `/tattle:report`, `/tattle:mute`, opt-in `/tattle:tell-claude`). 44 rules (`crates/checks/src/catalog.rs`).
- **Score:** 228 mutations; every rule that has one at precision and recall 1.00; 0 false alarms on clean sessions; deterministic. On all 163 labelled real sessions (Windows, 2026-11-05, 0.9.0 at 18e74c9): 0 of 112 honest sessions flagged, 23 of 49 false-claim sessions flagged by static checks (25 with test runs); `manifest.missing` left out (research-only, RESULTS §3s). Blind catch rate about 40% (11 of 28, Haiku 4.5). 224 everyday plugin sessions: false alerts fell from 8 in 40 to 0 in 36 (RESULTS §3k–§3q).
- **Main gaps:** "fixed" problems that never existed, wrong diagnoses (not statically checkable), code-level checks only for JS/TS, Claude Code only, no human labels yet.
- **Language:** Rust, one binary (`tattle`), Rust stable. Users install the plugin, never Rust tooling.
- **Test apps:** built outside this repo (the agent would see the traps).

## Progress log
Newest first.

### 2026-11-04 → 11-05 (build session 19 continued, cloud: release form, live Windows use, CI, 0.9.0)
- Release form: README at the root ("catch your AI agent's hallucinations", two real screenshots, blind-first headline, how results stay deterministic), SECURITY, CONTRIBUTING, MIT; private-only material out, checked by 7 privacy reviews of the `git archive` tree (the last two clean); maintainer-only files are `export-ignore`.
- Live Windows sessions (RESULTS §3r): 2 true alerts; 1 false (pasted code read as phantom references), fixed with tests.
- Report items say what was found, merge repeats and name the request (`Request N ("…")`); findings unchanged (same 37 on the 113 restored recordings).
- CI on macOS, Linux and Windows (clippy, tests, score, a plugin smoke test through the launcher); in the development repo on PR open and by hand, on the public repo on every push and PR. Its first Windows run found a real bug: the home folder was recorded when `HOME` and `USERPROFILE` differ. Fixed. Docker counts only with Linux containers.
- All 163 labelled sessions on Windows (RESULTS §3s, 0.9.0 code): 23/49 flagged by static checks (25 with test runs), 0/112 honest flagged; `manifest.missing` left out (empty `manifests/` folders in the Windows copy). `release_numbers.py --from`, synthetic line counted on its own.
- Version 0.9.0.

### 2026-11-04 (build session 19 continued, cloud: README reviews, mute and tell-claude, renamed tattle)
- README draft rewritten after 8 reviewer agents (first-time visitor, sceptic, growth, Hacker News, power user, security, technical writer, market); command line moved to `docs/CLI.md`.
- Security review found a real bug: deleted sessions' files stayed in packed snapshots. Fixed (repack before prune) with a test.
- Plugin: `/tattle:mute <rule>` / `/tattle:unmute`, and opt-in `/tattle:tell-claude on|off` (alerts handed back to Claude once per turn). Live-tested: Claude retracted a false "All tests pass." when told; a found bug (cv checked its own commands) fixed.
- Re-scored the 133 older labelled recordings, restored locally: one regression (H33) and one older miss (R52), both fixed; 41/41 checks at 1.00, 0/98 false alarms.
- **Renamed cv → tattle** (DECISIONS): binary, crate, plugin, commands, messages, `TATTLE_*` settings (old `CV_*` still work); `.checker/`, the recording format and rule ids unchanged.

### 2026-11-02 → 11-04 (build session 19 continued, cloud: everyday batches, MIT)
- MIT licence (`LICENSE`, every crate, plugin manifest).
- Everyday batches 1–6 through the installed plugin (`corpus/everyday/`, `scripts/everyday.py`): 224 ordinary sessions on this repo and zero-dependency JS, Python, Go, C and Ruby projects; ~$27. False alerts per batch: 8, 3, 2, 4, **0** (36 sessions, gate A2 met once in the cloud), 2. 0 true catches: no agent made a false claim. Every batch recorded before its fixes (RESULTS §3k–§3p).
- Fixes, all general, each with tests and a score run: more test runners and their summaries; test runs also recognised by their output; heredocs and newlines in commands; compound commands failing in a non-test part; quoted text and "lets tests pass"; names in any language; query parameters and flags; backward-compatible renames; made-up secrets; edits to the agent's own new test shown as a note (same finding id).
- Two fixes dropped a check's mutation recall and were caught by `cv score` and narrowed. Third-party projects were left out (installing their dependencies runs outside code).

### 2026-11-02 (build session 19, cloud: v1 release prep)
- Merged the plugin branch and the Docker/Windows branch into the release branch; fixed the merge gap (`claims.flaky_unobserved` had no plain sentence).
- Live plugin test in this cloud session: built `plugin-dist` locally (not pushed), installed it with the two `/plugin` commands, ran real sessions. Found and fixed: report noise from its own turn, `scope.unrelated_file` on a lone file, 6.5 s end-of-turn on a 20,000-file project (now ~2 s). Windows launcher for Git Bash.
- README rewritten around the plugin; `scripts/release_numbers.py` prints every README number from local recordings; release and CI workflows.
- 218 tests, clippy clean; synthetic 1.00/1.00, 0 false alarms, 233/233 deterministic. Real-session numbers come from `scripts/release_numbers.py`, which needs the local recordings.

### 2026-10-29 (build session 18, cloud: Docker backend)
- `crates/exec/src/docker.rs`: commands run in locked-down containers (no network, read-only root, no host home, limits, timeout kills the container). Used whenever there's no macOS sandbox. All 11 escape tests pass under Docker (run in this cloud session).
- `crates/exec/src/linuxdeps.rs`: a Linux dependency pool per lockfile, built only with `cv verify --allow-install` (download with scripts off, native builds with network off), mounted read-only into the containers. G2 end to end under Docker: 27/27 tests with no network; the whole verify pipeline, cross runs included, as expected.
- The Wine run of the harness tests hit the session's time limit before finishing; the other Windows test groups passed.

### 2026-10-29 (build session 18, cloud: Windows support)
- Development moved to Windows. The workspace now builds and tests there: Windows path forms, `USERPROFILE`/`USERNAME`, read-only git files, `restore` without `tar`, LF line endings (`.gitattributes`). Shared helpers in `capture::fsx`. Setup steps in `docs/WINDOWS.md`.
- Checked by cross-compiling and running the suite under Wine with Git for Windows; not yet on the real machine.
- `cv verify` doesn't run on Windows (macOS sandbox); recording live sessions on Windows is untested.

### 2026-10-29 (build session 17, cloud: plan #1/#2 mutations and docs; plan #4)
- Mutations and controls: `exec-fix-not-demonstrated` / `exec-fix-demonstrated`, `exec-tests-bent` / `exec-tests-kept`. Synthetic 1.00/1.00, 0 false alarms, 227/227 deterministic.
- R85 is handled (a test named as "now passes" counts as the fix claim). A wider bug-report rule for R84 was tried and taken back out.
- Docs: `DECISIONS`, `RESULTS` §3j, `PHASE_4B_PLAN`, `IMPROVEMENT_PLAN`, `BUILD_PLAN`.
- Plan #4 (static part): `claims.flaky_unobserved`, with 1 mutation and 2 controls at 1.00/1.00. Labels for R22, R52 and R89 updated in `scripts/make_labels.py` and `corpus/labels/`, not yet run on their recordings.

### 2026-10-29 (build session 17: plugin for everyday use, cloud)
- `plugin/`: install with two `/plugin` commands; hooks, `/cv:report`, launcher. `scripts/release_plugin.sh` builds the `plugin-dist` branch (must run on a Mac for macOS binaries; not run yet).
- End of every turn: all checks on the live session (`Recording::load_live`, identical to an export by test). New alerts are shown as one plain sentence; nothing when clean.
- `checks/plain.rs`: a plain sentence for every finding id; "tolerated-only" checks are notes.
- Housekeeping: 30 sessions / 30 days, home folder never recorded, no double recording, log rotation.
- Dogfooding on this repo (`.claude/settings.json`, `scripts/dev_plugin.sh`, `/cv-report`). The first live session found 2 checker gaps, both fixed with tests: `cargo test` wasn't a test run, and heredoc text was judged as commands.
- Synthetic score unchanged (1.00/1.00, 0 false alarms, 217/217).

### 2026-10-29 (build session 16: plan #1/#2 in progress; moved to the cloud)
- `crates/exec/src/cross.rs`: per-test results (vitest JSON reporter); new tests on the old code (P5) and old tests on the new code (P4), each twice; written as `record.cross` by `cv verify`.
- `exec.fix_not_demonstrated` (#1) and `exec.tests_bent` (#2) in `checks/exec_findings.rs`. First tries: R89 and R22 caught; R81 and R91 (genuine fixes) clean. R85 is missed: its closing message says "now passes", not "fixed". R84 is missed: its prompt isn't recognised as a bug report.
- **Not done in this session:** mutations and controls and docs (done in session 17); a full re-verify (needs the macOS sandbox and local recordings).
- Development continued in Claude Code's cloud sessions.

### 2026-10-27 (build session 15: Phase 4b.3)
- 4b.3: the app-start probe (P3) and `exec.app_does_not_start`. Unix sockets are allowed only inside the sandbox folders, and the temp root is short (socket path limit). Gate passed: both A1 Haiku runs flagged from turn 16; every reference server and the Opus run start; 0 false failures over 173 sessions. 2 new true catches (R109, R11 run 2).

### 2026-10-27 (build session 14: Phase 4b.1–4b.2)
- 4b.1: `cv verify` materialises snapshots and gets dependencies without network (link, or offline cache plus same-version native builds). 163/163 recorded sessions runnable.
- 4b.2: P0–P2 probes (baseline, final ×2, build/type check) write an execution record; `cv check` adds `exec.regression`, `exec.tests_fail`, `exec.build_fails` and `exec.claim_contradicted`. 5 of 6 gate sessions caught (A1's dead app needs 4b.3), 0 false failures over 173 sessions vs the oracle. 5 mutations; escape tests serialized.

### 2026-10-27 (build session 13: Phase 4b.0 sandbox spike)
- `crates/exec`: `sandbox-exec` profile and runner (no external network, writes only in the copy, secrets unreadable, scrubbed env, process-group timeout), plus `link_node_modules` and Docker arguments.
- 10 escape tests pass, including a real vitest suite inside the sandbox. Found a network-rule trap (a broad local-ip rule with Unix sockets allowed outbound traffic), now fixed and tested.

### 2026-10-27 (build session 12: plan D run)
- Plan D ran in full for $228: 8 medium builds, the flagship twice on Haiku 4.5 (it broke the app at turn 16 both times and kept claiming done), the simulated user (21 false "done" turns), and Opus (78/79, out of budget).
- Long builds exposed new false-alarm sources, all fixed with tests. 45 → 3 findings on B1–B5; spec names, hand-written routes, units, whole-session manifests, hand-rolled test functions. Mid-session "no new deps" rules are now judged from their turn. The gate uses the best pass rate.
- Biggest gap: false "done" claims on broken apps are behavioural; they need Phase 4b execution. Recorder: the secret guard blocked 2 exports (test credentials), as designed.

### 2026-10-26 (build session 11: big tests, recordings out of git)
- `claims.fix_unreproduced` and `claims.tests_pass_other_command`, each with mutations and controls. Held-out batch 3 (30 blind prompts, 8 with real bugs): `cv` caught 6 of 15 hallucinating sessions, 0 false alarms on genuine fixes; `fix_unreproduced` caught 1 of 7 fake fixes. Measured, not fixed yet.
- Recordings are out of git. Labels moved to `corpus/labels/`, plus a real-session harness test.
- `bigtests/` suites A–D: specs, 21-turn and 5–6-turn scripts, 423 hidden acceptance tests, references passing them all, runners (`bigtest.py`, `acceptance.py`, `fake_claude.py`), plan D (about $90–130). Validated, **not run**.

### 2026-10-26 (build session 10: recall study)
- 100 trap sessions: 20 on Haiku 5.5, 80 on Haiku 4.5. First real hallucinations: 34 sessions, 21 high or medium.
- Before the fixes, `cv` raised 61 false findings on the first 70 sessions and caught almost nothing. After: 4 new checks, 7 false-alarm fixes, 0 false alarms on all 133 real sessions.
- Real sessions became a labelled test suite (`labels.json`, `scripts/make_labels.py`) that `cargo test` enforces. Results are no longer committed (user decision).
- Held-out retest (30 prompts written by an agent that couldn't see the checker): `cv` caught 5 of 13 hallucinating sessions, with 2 false findings in 30.

### 2026-10-25 (build session 9: fresh-batch false alarms)
- Fixed the 5 false-alarm causes from the first fresh batch (renamed tests, rename field check, phrase boundaries, manifest test paths, negated routes); lexicon v3. Synthetic still 1.00/1.00. 147 tests, clippy clean.
- `docs/KNOWLEDGE.md`: structured knowledge base
- Second fresh batch: 8 new manifest sessions, independent reviewer batch D. Result: 2 findings (1 false, 1 grey), down from 8 false in 6; requests confirmed 56% rules / 94% with manifest; 0 real problems missed. Written up in `docs/RESULTS.md` §3d.

### 2026-10-23 (build session 8: Phase 6 and real-data evaluation)
- 6a intent checks and 6b agent manifest (hook-requested, statically verified); 8 intent + 6 manifest mutations and controls; 27 checks at 1.00/1.00 synthetic
- 18 more real sessions (12 + 6 fresh) with manifest on held-out prompts; 30 constraint-focused held-out prompts; 3 AI reviewer batches over all 25 sessions
- Real-data false alarms fixed (about 20 → 2 on the first 19); a fresh batch still showed 8 false of 10
- Compiled `docs/RESULTS.md`

### 2026-10-23 (build session 7: Phase 5)
- `checks/intent.rs`: prompt → intent facts (lexicon v1, then v2), repo grounding, anaphora, turn merging, coverage accounting. `cv intent`.
- `harness/intent_score.rs`: scoring against expected facts; `cv score` reports corpus, dev and held-out sets, with a per-kind breakdown
- Held-out sets written by separate agents that never saw the lexicon. v1: corpus 0.98 but held-out 0.57 (6 invented). v2 (tuned on that set, now `corpus/dev`): 0.89 there, but **0.49 / 12 invented** on a fresh held-out set.
- Conclusion: a fixed lexicon is precise only for some kinds. That decides Phase 6's design.
- 135 tests, clippy clean

### 2026-10-22 (build session 6: first real recordings)
- `cv check`, `scripts/record_headless.py` (headless Claude Code runner with a command allow-list)
- Recorded G1–G3, then froze G1/G3 as bases (+ seeded T1) and recorded E2, E3, T1, V1. ~$4.30 in total.
- Real data corrected the hook format: no exit codes, failures via `PostToolUseFailure`, `tool_use_id` pairing, `Stop.last_assistant_message`, task notices as prompts, denials fire no hook. Recorder, adapter, synthetic sessions and mutations updated.
- Two false alarms fixed (documented empty catch; wildcard reads). Transcripts slimmed (~400 → ~25 KB). Export bug with relative paths fixed.
- All 7 real recordings: no findings, which matches a manual read. Synthetic score unchanged (all 16 checks with mutations at 1.00/1.00). 116 tests.

### 2026-10-22 (build session 5)
- Phase 4: `checks/ts.rs` (tree-sitter code facts) and `checks/code.rs` (8 code checks comparing each file before/after a turn)
- Found by scoring: `scope.unrelated_file` fired twice when the real change was also purely additive. Rule tightened (the unused code must be the file's only change).
- Faster: batched file reads, git grep lookups (~270 → ~90 ms per recording)
- Score: all 16 checks with mutations at precision/recall 1.00; 0 false alarms; 80/80 deterministic. Saved as `harness/results/2026-10-22-453d131.json`.
- 109 tests, clippy clean

### 2026-10-22 (build session 4)
- Rest of Phase 3, mutation first each time: `events.edited_without_reading`, `tests.not_rerun_after_edit`, `commands.risky`, `events.edit_outside_project`
- `checks/imports.rs`: static relative-import scan, so "tests not re-run" only fires for code the tests actually use
- Two fixes found by unit tests: the recursive-`rm` regex missed `rm -rf ~/Documents` (replaced with a small parser); the `sudo` rule fired on text inside quotes
- Score: 10 checks at precision/recall 1.00 on 75 mutations, 0 false alarms, 80/80 deterministic. Saved as `harness/results/2026-10-22-f407080.json`.
- 97 tests, clippy clean. Harness tests take ~50 s in debug builds.

### 2026-10-22 (build session 3)
- Phase 2 adapter (`checks/events.rs`) and the first behaviour checks (`checks/behaviour.rs`): sequence gaps, recorder problems, changes made by commands, unexplained changes, tests edited after failure, false "tests pass" claims
- Two false-alarm probe sessions added to the synthetic set (Vite scaffold; formatter + hedged ending). Both stay at 0 findings.
- Score: 4 checks with mutations at precision 1.00 / recall 1.00; 0 false alarms; 61/61 deterministic. Saved as `harness/results/2026-10-22-0ad00cb.json`.
- 82 tests, clippy clean. Check rules recorded in `docs/DECISIONS.md`.

### 2026-10-22 (build session 2)
- Phase 0.5 and 0.6 done: `crates/checks` (recording loader, finding catalog, placeholder checker) and `crates/harness` (3 synthetic sessions, 12 mutations, scorer)
- New commands: `cv score [--save] [--json] [--keep]`, `cv synth`, `cv mutate`
- Two mutation realism fixes found by inspecting output: `delete-test` only deletes pre-existing tests; `test-edit-after-fail` uses a realistic fail → edit → pass order
- Baseline saved: `harness/results/2026-10-22-e3f33fb.json` (35 mutations, recall 0, 0 false alarms, 38/38 deterministic, ~10 s per full run)
- 68 tests, clippy clean

### 2026-10-22 (build session 1)
- Rust workspace set up (`crates/cv`, `crates/capture`)
- Phase 0.2 recorder built: `cv hook`, `cv record list | export | restore`, `cv sandbox init`
- 43 tests: 21 unit, 18 end-to-end through the real binary, 4 corpus checks. Clippy clean.
- Measured: 2 MB release binary, ~4 ms per tool event, ~60 ms per snapshot
- Corpus: 15 prompt files in `corpus/prompts/`. G3, E3, T1 and V2 switched to TypeScript; Python versions kept for later. G3 added to the first batch (E3 and T1 need it as a base): 14 sessions.
- All remaining proposals decided: sandboxed execution (Phase 4b), JS/TS + Vitest/Jest first, recording storage (only what's needed, ≤ 500 KB per run)
- Known limits recorded in `docs/DECISIONS.md` → "Recorder design"

### 2026-10-22
- Design discussion captured in `docs/DESIGN_NOTES.md`
- Build plan written (`docs/BUILD_PLAN.md`): test-first, phases 0–8 with scoring gates
- Test corpus v1 written (`docs/TEST_PROMPTS.md`): 15 prompts across greenfield, edits, traps and vague controls
- Local git repo created at `~/Code/Code_Validation` (no remote). Moved from `~/Documents/Code_Validation` because macOS blocks the Claude Code helper from accessing Documents (see `docs/DECISIONS.md`).
- Decided: Rust single binary delivered through a Claude Code plugin / npm / Homebrew, Claude Code only, no AI in the checker, staged labelling (12 sessions first), test apps built outside the repo
- (All decided in build session 1, see above)
