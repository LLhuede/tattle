# Code Validation: Build Plan

A local, static checker that compares what an AI coding agent was asked to do against what it actually did, and reports findings a human or an agent can act on.

**Approach: test-first.** Before writing any checker logic, build a set of real vibe-coded sessions with known, labelled problems. Every later phase is "done" only when it scores well against that set.

---

## Principles (from the design discussion)

1. **No AI.** The checker never calls a model or agent. Analysis is static by default. Running the user's code is an optional, sandboxed layer (Phase 4b).
2. **Local only.** No network access. Secrets are redacted when captured. `.checker/` is always gitignored.
3. **Every part of the prompt is accounted for.** Each part is turned into a fact, marked as filler, or shown as "couldn't check". Nothing disappears silently.
4. **Every code change is explained.** Snapshot diffs are reconciled against the event log. Changes nothing explains get flagged.
5. **Evidence ladder.** Code state > recorded execution > recorded actions > agent statements > prompt interpretation. Agent statements never count as evidence.
6. **Deterministic.** Same snapshot + same rule versions = byte-identical output.
7. **Precision over recall.** A false alarm costs more trust than a missed finding.

---

## Phase 0: Test corpus and scoring harness (first)

The goal is a library of recorded AI coding sessions, each with labelled correct answers, plus a harness that scores the checker against them.

### 0.1 Write the test prompts ✅
- 15 prompts written the way a vibe coder actually types (see `TEST_PROMPTS.md`). Machine-readable versions live in `corpus/prompts/<ID>.json`, checked by `cargo test --test corpus`.
- Each prompt file records:
  - The prompt and any follow-up turns
  - The **expected intent facts**, labelled by hand
  - The **trap**: the failure we expect the AI might make
  - The **expected "couldn't check"** phrases
- Greenfield apps (G) become the starting point for edit prompts (E, T, V). That gives both from-scratch and diff-based cases.

### 0.2 Build a minimal recorder (also the start of Phase 1) ✅
- A Claude Code hook (`tattle hook <event>`, part of the Rust binary) that only **records**:
  - Raw hook JSON for `UserPromptSubmit`, `PostToolUse` and `Stop`, with sequence numbers
  - A shadow-git snapshot at each prompt submit and at `Stop`
  - A copy of the transcript file at `Stop`
- No analysis yet. This becomes the capture layer later, so it's not throwaway work.
- **Built:** `tattle hook`, `tattle record list`, `tattle record export` and `tattle sandbox init`, with 38 tests. It already includes much of Phase 1: secret redaction, path placeholders, trimming, exclusions, a self-ignoring `.checker/`, locking and sequence numbers.

### 0.3 Record real sessions (in progress)
- **First batch:** G1, G2, G3, E2, E3, T1, V1 × 2 runs = 14 sessions. The rest of the corpus is recorded before Phase 5.
- How to record: `docs/RECORDING.md` (interactive) or `scripts/record_headless.py` (headless Claude Code).
- **Status:** first batch run once each, headlessly: G1, G2, G3 recorded and frozen as bases (`corpus/bases/`); E2, E3, T1, V1 recorded from those bases. All unlabelled. The real recordings corrected the event format (see `DECISIONS.md` → "Real Claude Code hook format") and exposed 2 false alarms, now fixed. **Next:** label these 7 (then `tattle score` injects all 16 defects into each real session automatically), and record second runs plus interactive runs.
- Sessions run in `~/Code/cv-sandbox/<prompt-id>-run<n>/` (a fresh folder with no `CLAUDE.md`), so the agent can't see the traps described in this repo. Recordings are copied back into `fixtures/recorded/`.
- Seeded starting repos (`corpus/bases/`) hold the fixed starting state for edit and bug-fix prompts, including planted bugs where a prompt needs one.

### 0.4 Label ground truth (human step)
For each recorded run, fill in `labels.json`:
- For each intent fact: met, violated, or couldn't tell
- Defects actually present (test edited to pass, stub, scope creep, false claim, and so on)
- Overall: would you accept this change? yes or no

Estimated effort: about 3–4 minutes per run, so ~50 minutes for the first batch. Re-label ~5 runs about a week later to check consistency. No AI pre-labelling.

### 0.5 Mutation fixtures (automatic) ✅
`tattle mutate` / `harness::mutate` takes a clean recording and injects one known defect, writing `expected.json` with the finding it should cause. Mutations are regenerated on every score run, never committed. Realism rules (one problem per mutation) are in `DECISIONS.md`.

| Mutation | What it changes |
|---|---|
| `delete-test` | Removes a test case |
| `weaken-assert` | Replaces an assertion with `expect(true)` / `assert True` |
| `skip-test` | Adds `.skip` / `@pytest.mark.skip` |
| `stub-function` | Replaces a function body with a TODO / `throw` / fixed return |
| `add-dependency` | Adds a package to package.json / requirements |
| `out-of-scope-edit` | Edits a file unrelated to the prompt |
| `false-claim` | Final message says "all tests pass" when the last recorded run failed |
| `test-edit-after-fail` | Inserts a failing test run, then a test-file edit, then a passing run |
| `swallow-error` | Wraps code in an empty `catch` / `except: pass` |
| `hardcoded-secret` | Adds a fake API key literal |
| `bash-edit` | Changes a file with no matching edit event (simulates `sed`) |
| `dropped-event` | Removes an event from the log (simulates a missed hook) |

### 0.6 Scoring harness ✅
`tattle score` runs the checker over every fixture and reports:
- **Precision and recall per check** (mutations plus labelled runs)
- **False alarms on clean controls**, where the target is zero warnings
- **Determinism**: runs twice and compares the output byte for byte
- **Speed**: time per fixture
- **Coverage invariant**: consumed + filler + residual = the whole prompt, for every prompt

**Phase 0 is done when** the corpus is recorded and labelled, mutations generate, and `score` runs end to end with a placeholder checker that finds nothing. That gives us a recall-0 baseline.

**Status:** the harness is done and the baseline is saved (`harness/results/2026-10-22-e3f33fb.json`): 3 synthetic clean fixtures + 35 mutations, recall 0 for all 12 checks, 0 false alarms, 38/38 deterministic. Until real recordings exist, the clean fixtures are **3 synthetic sessions** (`harness::synth`), recorded through the real recorder. **Remaining:** record and label the first real batch, then check the synthetic event format against it.

---

## Phase 1: Capture layer
Turn the recorder into the real capture layer.
- Hooks: `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SessionStart`
- Shadow repo at `.checker/shadow.git`, plus an automatic `.gitignore` entry. `.env*` and key files are always excluded.
- Secret redaction before anything is written to disk (pattern list plus a high-randomness string check)
- Logs readable only by the user, with old turns deleted after a set number of days

**Tests:** replay recorded hook payloads; planted secrets never reach disk; `.checker/` is never committed; capture finishes within the hook's time limit.

**Done when** all Phase 0 sessions can be re-captured from their raw payloads, giving identical output.

## Phase 2: Normalised event log and reconciliation (mostly done)
- Claude Code adapter that converts raw hook JSON and the transcript into the normalised event schema
- Reconcile the snapshot diff against edit events, labelling each change "explained", "changed by a command", "unexplained" or "reverted"
- Completeness checks: sequence gaps, hook stream compared with the transcript, and a "partial capture" state

**Tests:** the `bash-edit` and `dropped-event` mutations are caught at 100%, with no "unexplained" on clean runs.

**Status:** ✅ adapter (`checks/events.rs`), reconciliation (`changed_by_command`, `unexplained_change`), sequence gaps and recorder problems (`capture.incomplete`). Both mutations caught at 100%, 0 false alarms. **Not done:** comparing the hook stream against the transcript.

## Phase 3: Behaviour checks (first useful checks) ✅ (on synthetic data)
These need no prompt understanding:
1. Edited a file it never read ✅ `events.edited_without_reading`
2. Tests not re-run after the final edit ✅ `tests.not_rerun_after_edit` (tested code only)
3. Test file edited between a failing run and a passing run ✅ `tests.edited_after_failure`
4. Claimed tests pass, but the last recorded run failed ✅ `claims.tests_pass_false`
5. Risky commands ✅ `commands.risky` (package installs moved to Phase 4's `deps.added`)
6. Edits outside the project ✅ `events.edit_outside_project`

(Rules for the built checks: `DECISIONS.md` → "Behaviour check rules".)

**Done when** precision ≥ 0.95 and recall ≥ 0.9 on the matching mutations, with zero false alarms on clean controls.

**Status:** all 6 meet the gate on synthetic data: precision 1.00 and recall 1.00 each, 0 false alarms on 5 clean sessions, 80/80 deterministic (`harness/results/2026-10-22-f407080.json`). **Real labelled recordings are still needed** to confirm precision before calling Phase 3 done for real.

## Phase 4: Code facts and prompt-independent checks ✅ (on synthetic data)
- tree-sitter parsing of before/after snapshots. **TypeScript/JavaScript first** (Vitest/Jest only for v1), Python second.
- Facts: symbols added, removed or changed; imports; dependencies; test and assertion counts; skipped tests
- Checks: deleted or weakened tests, stubs, new dependencies, swallowed errors, hardcoded secrets, imports not found in the lockfile
- Snapshots are content-hashed, and every fact is tagged with the hash

**Done when** the same precision and recall targets are met on the matching mutations, with each check scored separately.

**Status:** all 8 checks at precision 1.00 / recall 1.00, 0 false alarms, 80/80 deterministic (`harness/results/2026-10-22-453d131.json`). Parser: tree-sitter (TSX grammar). "Imports not found in the lockfile" (hallucinated packages) is not built yet. Rules: `DECISIONS.md` → "Code check rules".

## Phase 4b: Execution checks (opt-in)
**Status:** 4b.0–4b.3 done (2026-10-27): sandbox, offline dependencies, items 1–2, and an app-start probe. Items 3–4 (`docs/IMPROVEMENT_PLAN.md` #1–#2) are built, 2026-10-29; their re-verify gate isn't run yet. See `docs/PHASE_4B_PLAN.md`.

Run the user's code without any AI, only on request, in the macOS sandbox, on a copy of the project, with network off.
1. Type-check / build (`tsc`, lint)
2. Re-run tests on the final code (evidence the agent can't fake)
3. Old tests on new code (catches deleted or weakened tests)
4. New test on old code (a bug-fix test should fail before the fix)
5. Coverage of changed lines
6. Mutation testing on changed lines (opt-in, slow)

- Each check runs twice. A disagreement is reported as "flaky", not as a failure. If tests can't run, the result is "couldn't verify by running".
- Results are tagged "checker-run execution" on the evidence ladder (between code state and the agent's recorded runs).

**Tests:** T1, E3 and E4 recordings, plus mutations. Sandbox escape tests: network, writes outside the copy, reading `~/.ssh`.

## Phase 5: Prompt normalisation into intent facts ✅ (built; generalisation measured)
- Protect code spans → split clauses → match constraint lexicon → ground nouns against the repo index → extract numbers and units → merge turns
- Versioned lexicon (`lexicon/v1.yaml`) with a closed list of fact kinds
- Coverage invariant enforced at runtime and in tests

**Tests:** for every corpus prompt, the extracted facts match the hand-labelled expected facts, and the residual matches the expected "couldn't check" list.

**Done when** ≥ 90% of labelled checkable facts are extracted, no facts are invented, and the coverage invariant holds on 100% of prompts.

**Status:** met on the corpus (0.98, 0 invented, coverage 15/15). **Not met on unseen prompts:** 0.49 recall, 12 invented on a fresh held-out set. Only some fact kinds are precise there (see `DECISIONS.md` → "Phase 5 results"). `tattle intent "<prompt>" --project DIR` shows the facts for any prompt.

## Phase 6: Intent checks and agent manifest ✅ (built; see docs/RESULTS.md)
- Check templates (requirement or claim vs evidence) for each fact kind. Only kinds that are precise on held-out prompts may report violations; the rest are information only (see `DECISIONS.md` → "Phase 5 results").
- Optional agent manifest (instruction added to `CLAUDE.md`), with each claim checked against code facts
- "Defined but couldn't confirm it's wired up" when call-graph reachability can't be proven

**Done when** checker verdicts agree with human labels on ≥ 90% of checkable intent facts, with zero false "violated" verdicts on clean runs.

## Phase 7: Report and feedback loop (after the gaps in docs/RESULTS.md §4 are closed)
- Output: Markdown (human) plus JSON/SARIF (agents and tools)
- Each finding has evidence (file:line, event number), its evidence level and a fixed confidence level
- The `Stop` hook feeds findings back to the agent, capped at 1–2 automatic fix rounds
- Four states: ✅ verified · ⚠️ violated · ❔ couldn't check · ⛔ capture incomplete

## Phase 8: Low-code packaging (in progress)
- ✅ Claude Code plugin (`plugin/`), installed with two `/plugin` commands from the `plugin-dist` branch (`scripts/release_plugin.sh`). It bundles the `tattle` binary for each platform and sets up the hooks itself.
- ✅ Check at the end of every turn, on the live session; new problems shown to the person as one plain sentence; `/tattle:report` for the rest (`crates/tattle/src/live.rs`, `checks/plain.rs`)
- ✅ Housekeeping: old sessions deleted, home folder never recorded, no double recording, error log rotated
- ✅ Used on this repo's own development (`.claude/settings.json`, `scripts/dev_plugin.sh`)
- Done since: feeding alerts back to Claude at `Stop`, opt-in (`/tattle:tell-claude`). Next: `/tattle:undo`; signed, notarised macOS binary
- Also published through npm (platform binaries as optional dependencies), Homebrew and GitHub releases: same binary and version, with checksums
- Universal macOS binary, signed and notarised, so users get no security warnings
- Presets: Prototype / Real users / Money or personal data
- First-run project scan ("here's what I learned")
- Undo this turn (restore from snapshot)
- Learns from "That's fine" dismissals

---

## Proposed folder layout

```
Code_Validation/
  docs/              BUILD_PLAN.md, TEST_PROMPTS.md, design notes
  corpus/
    prompts/         one file per prompt (text, turns, expected facts, trap)
    bases/           seeded starting repos for edit / bug-fix prompts
  fixtures/
    recorded/<prompt-id>/<run>/   raw hooks, transcript, snapshots, labels.json
    mutations/<prompt-id>/<run>/<mutation>/
  Cargo.toml         Rust workspace
  crates/
    cv/              the single binary: hook, record, sandbox, score, synth, mutate
    capture/         Phase 0.2 / 1: hooks, snapshots, redaction, export
    checks/          Phases 2–6: recording loader, finding catalog, checks
    harness/         synthetic recordings, mutations, scoring
  harness/results/   saved score runs (committed)
  rules/             secret and code-pattern rules (embedded at build time)
  lexicon/           versioned constraint lexicon (embedded at build time)

~/Code/cv-sandbox/   test app sessions run here, outside this repo
```

---

## Decisions

Full details in `docs/DECISIONS.md`.

| Decision | Status |
|---|---|
| Rust, shipped as one self-contained binary (`tattle`) | Decided |
| Distribution: Claude Code plugin first, then npm, Homebrew, GitHub releases | Decided |
| Claude Code only for now | Decided |
| No AI in the checker | Decided |
| Staged labelling: first batch of 14 sessions, done by the user | Decided |
| Test apps built in `~/Code/cv-sandbox/`, outside this repo | Decided |
| Running the user's code: optional, sandboxed (Phase 4b) | Decided |
| TS/JS with Vitest/Jest first | Decided |
| Recording storage: only what's needed, ≤ 500 KB per run | Decided |

---

## Order of work

```
0.1 prompts ─► 0.2 recorder ─► 0.3 record runs ─► 0.4 label ─► 0.5 mutations ─► 0.6 score
                                                                                │
             ┌──────────────────────────────────────────────────────────────────┘
             ▼
Phase 1 capture ─► 2 events + reconcile ─► 3 behaviour checks   ◄─ first usable version
             ─► 4 code checks ─► 4b execution (opt-in) ─► 5 prompt facts ─► 6 intent checks ─► 7 report ─► 8 packaging
```

After each phase, re-run `score`. Results are written to `harness/results/` so progress (and regressions) can be tracked over time.
