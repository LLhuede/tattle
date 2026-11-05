# Knowledge base

Everything learned building `tattle` so far, organised for someone (human or agent) picking up the work. Facts are verified against real data unless marked otherwise.

- Latest numbers: `docs/RESULTS.md`
- Reasons behind each choice: `docs/DECISIONS.md`
- Phase plan: `docs/BUILD_PLAN.md`

Updated 2026-10-25.

---

## 1. What the project is

A local checker that compares what a human asked an AI coding agent to do with what the agent actually did, and reports findings a human or an agent can act on.

**Hard rules**
- **No AI in the checker.** It never calls a model or agent. Running the user's code is allowed only in an optional, sandboxed layer (Phase 4b, not built).
- **Local only.** No network, no git remote. Secrets are redacted before anything is written.
- **Deterministic:** same input + same rule versions = byte-identical output
- **Precision over recall.** A false alarm costs more trust than a miss.
- **Test first:** every check gets a mutation before it gets code

**Shape:** one Rust binary, `tattle` (~4 MB, Apple Silicon), in four crates:
- `capture`: recording and export
- `checks`: the checker
- `harness`: synthetic sessions, mutations, scoring
- `tattle`: the command line

Distribution later: a Claude Code plugin first, then npm, Homebrew and GitHub releases.

---

## 2. How it works (pipeline)

```
Claude Code hooks ──► tattle hook (record, redact, snapshot) ──► .checker/ in the project
                                                                │
                              tattle record export ◄────────────────┘
                                     │  (slimmed, ≤ 500 KB)
                                     ▼
                          recording folder (events, transcript, snapshots.bundle, manifests)
                                     │
                                  tattle check
          ┌──────────────┬───────────┴───────────┬──────────────────┐
     behaviour (P2–3)  code (P4)        intent (P5 → P6a)     manifest (P6b)
     event log         before/after      prompt → facts →      agent's claims →
                       each turn         verdict vs code       verified vs code
```

| Phase | What | Where |
|---|---|---|
| 0 | Test corpus, recorder, mutations, scoring | `corpus/`, `crates/capture`, `crates/harness` |
| 2 | Event adapter, reconciliation, completeness | `checks/events.rs`, `checks/behaviour.rs` |
| 3 | Behaviour checks | `checks/behaviour.rs` |
| 4 | Code checks (tree-sitter) | `checks/ts.rs`, `checks/code.rs`, `checks/imports.rs` |
| 5 | Prompt → intent facts (lexicon, now v3) | `checks/intent.rs` |
| 6a | Intent facts → verdicts against the code | `checks/intent_checks.rs` |
| 6b | Agent manifest, verified against the code | `checks/manifest.rs`; request in `capture/hook.rs` |

**Commands**
- `tattle hook`: called by hooks; always exits 0, prints nothing (except in manifest mode, and the end-of-turn line in plugin mode)
- `tattle report [--session ID]`: the session's findings in plain words (the plugin's `/tattle:report`)
- `tattle sandbox init [--base] [--manifest]`
- `tattle record export | restore | list`
- `tattle check <recording> [--json]`
- `tattle intent "<prompt>" [--project]`
- `tattle score [--save]`
- `tattle synth`, `tattle mutate`
- Scripts: `scripts/record_headless.py`, `scripts/run_batch.sh`, `scripts/analyse_runs.py`

---

## 3. Claude Code facts (verified against real recordings, v2.1.304–2.1.307)

### Hooks and events
| Fact | Consequence |
|---|---|
| `PostToolUse` fires only when a tool **succeeded**. A non-zero exit fires **`PostToolUseFailure`** with `error` starting `Exit code N`. | Subscribe to `PostToolUseFailure`, or failing test runs are invisible. A Bash `PostToolUse` means exit 0. |
| Bash `tool_response` has `stdout`, `stderr`, `interrupted`, `isImage`, `noOutputExpected`: **no exit code** | Never rely on an exit-code field |
| Every tool call has a **`tool_use_id`** shared by its Pre and Post events | Pair calls exactly; mark failed calls on both events |
| `Stop` carries **`last_assistant_message`** | Best source for "tests pass" claims |
| **Background-task notices arrive as `UserPromptSubmit`** (`<task-notification>…`) | They are not human prompts; turns start only at human prompts |
| **Headless permission denials fire no hook** (not even `PermissionDenied`) | The transcript's `tool_result` with `is_error` is the second source |
| `SubagentStop` has `agent_transcript_path` | Copy subagent transcripts |
| Hooks run with `CLAUDE_PROJECT_DIR` set | Use it as the project root |
| `UserPromptSubmit` hook stdout (exit 0) is added to the agent's context; a `Stop` hook can print `{"decision":"block","reason":…}` once (`stop_hook_active` prevents loops) | This is how the manifest is requested with no user typing |
| Exit code 2 from a `PreToolUse` hook **blocks the tool** | `tattle hook` handles itself before argument parsing and always exits 0 |
| Hook settings in `.claude/settings.local.json` load in headless (`claude -p`) mode too | The sandbox setup works for both |
| A `Stop` hook's `{"systemMessage": …}` is shown to the person, not to Claude (docs, v2.1.312) | The plugin's end-of-turn line uses it |
| Cloud sessions don't load plugins (not even ones a repo enables), but do run a repo's `.claude/settings.json` hooks; Claude Code picks up a settings file created mid-session | This repo dogfoods through project hooks |
| Skills get `${CLAUDE_SESSION_ID}`, `${CLAUDE_PLUGIN_ROOT}` and `${CLAUDE_PROJECT_DIR}` substituted; `` !`cmd` `` lines need a matching `allowed-tools` rule | `/tattle:report` reports the exact current session |

### Transcripts
- ~400 KB per session, mostly bookkeeping (`attachment`, `api-request-shape`, file history)
- Export keeps only user/assistant/system messages, tool calls with shortened input, and tool results' id + `is_error`, which is ~25 KB. The agent's reasoning is reduced to its length.

### Agent behaviour (Claude, headless, small TS/JS apps)
- Did what was asked in **102 of 104** requests across 25 sessions; avoided every trap; when a test failed it **fixed the code, not the test**; it was honest when tests couldn't run
- Often pipes output through `tail` (which hides exit codes), runs `npm ci`, updates READMEs, and uses `sed` for multi-file renames
- Chose JavaScript when the prompt didn't say TypeScript
- Writes the manifest every time it's asked (18/18), and the manifests were honest

### Environment gotchas
- **macOS privacy:** the Claude Code helper (`~/Library/Application Support/Claude/claude-code/<version>/claude.app`) is its own app and can't read `~/Documents`. Keep projects outside Documents/Desktop/Downloads. Full Disk Access would break on every update, because the path contains the version.
- **zsh:** `--include=*.js` and `$var:x` get mangled; quote globs and use `${var}`
- **Never rebuild `target/release/tattle` while recording sessions use it.** The runner now freezes a copy per binary hash in `~/Code/cv-sandbox/_bin/`.
- **Usage limits cut headless runs short** ("You've hit your session limit"). Check `is_error`, and discard and re-record those runs.
- Several runs started at once can race on shared temp files; use per-process temp names

---

## 4. What works (measured)

| Area | Result | Source |
|---|---|---|
| Injected-defect detection | 27 checks, 140 mutations: precision and recall **1.00**, 0 false alarms, deterministic | `tattle score` |
| Manifest | Written 26/26; **568/570** claim items check out; AI reviewers found 0 false claims | `runs-*.json`, `fixtures/reviews/` |
| Manifest coverage | 33 sessions. Requests confirmed in code: **53% → 89%** with the manifest (second fresh batch: 56% → 94%) | `runs-*.json` |
| Real sessions as a test suite | 133 labelled sessions, **0 false alarms**, all statically checkable hallucinations caught; enforced by `cargo test` | `labels.json`, `scripts/make_labels.py` |
| Held-out hallucination catch rate | 5 of 13 sessions (4 of 10 high/medium); 2 false findings in 30 sessions | RESULTS §3e |
| False alarms on fresh real sessions | **0.13 per session** after the fix (1 false, 1 grey of 2 findings over 8 sessions), down from 1.33 | `fixtures/reviews/` batch D |
| Prompt understanding, corpus | 0.98 recall | (tuned; optimistic) |
| Coverage invariant (every word accounted for) | Holds on all 85 prompts | `tattle score` |
| Speed | ~4 ms per tool event, ~60 ms per snapshot, ~150 ms per `tattle check` | |

## 5. What doesn't work yet (gaps)

1. **False alarms on new real sessions: mostly fixed.** On the first fresh batch, 8 of 10 findings were false. The causes were renamed tests, a literal rename check, phrase boundaries, manifest test paths and negated routes, all fixed in lexicon v3. The second fresh batch had 1 false finding in 8 sessions. Its cause: the rename check matches the field on *any* object (H30: `amount` on settle-up transfers), not just the one the user named.
2. **Catching real mistakes: about 40% on held-out sessions** (recall study, RESULTS §3e). Caught: false test claims, contradictory manifests, references to things that don't exist, half-done renames. Missed:
   - "fixes" to problems that never existed (the next check to build)
   - claims made by running a different test command
   - wrong diagnoses
2b. **Manifest summary and intent verdicts don't share evidence.** A manifest requirement shows as unbacked even when the checker's own intent verdict proves it (e.g. "dont touch code.ts").
3. **Prompt understanding finds ~56%** of requests on unseen prompts:
   - feature names 43%
   - data entities 0%
   - "add tests for X" 33%
   - `target.modify` invents more than it finds
4. **Remaining precision errors:** hedges on scope/numbers ("if possible", "ideally"), am/pm, constraints lifted in a later turn
5. **Many requests get no verdict;** some ✅s rest on weak evidence (the word appears; help text instead of logic)
6. **The manifest is checked structurally**, not behaviourally (a file changed, a name exists)
7. **No mutation yet** for `capture.incomplete`, `events.unexplained_change`, `intent.target_not_changed`
8. **Narrow evidence:** one agent, three small apps, AI-written prompts, AI reviewers, no human labels yet

---

## 6. Lessons (the transferable ones)

### About measuring
1. **A score on data you built against is optimistic.** Corpus 0.98 → first unseen set 0.57 → after tuning on it, a fresh set scored 0.49. Always keep a held-out set written by someone (or an agent) that never saw the rules.
2. **Held-out protocol:** when you tune on held-out failures, that set becomes dev and a new held-out set is needed. Fixing *how a fact is matched* (scoring bugs) is fine anytime.
3. **Synthetic perfection means little on its own.** Every check scored 1.00 on mutations while real sessions produced ~20 false alarms. Real sessions contain situations nobody thinks to synthesise: renames, reversals, multi-turn changes of mind, CSS names, plain-Node servers.
4. **Fresh-batch rule:** record a batch's numbers *before* fixing what it exposed; the next honest number needs a new batch
5. **Independent review works:** reviewer agents that judge a session *before* reading the checker's output give a usable precision estimate. Keep their judgements separate from human labels.
6. **Count invented facts per kind, not just recall.** The per-kind table decided the Phase 6 design (only precise kinds may report violations).
7. **When the agent makes no mistakes, you can't measure recall on real data.** Design sessions that make agents fail.

### About the design
8. **Let the agent interpret; verify statically.** The coding agent already understands the prompt. Asking it for a manifest of *checkable* claims (files, names, routes, tests) and verifying each one gets far more coverage than any fixed phrase list, without putting AI in the checker.
9. **A fixed phrase list plateaus around half recall** on unseen prompts. Keep it for constraints (no new packages, don't touch X, only change Y, tests pass, renames, routes, numbers) and let the manifest cover features and entities.
10. **Judge only what the session introduced, and only if it survived:** compare before and after each turn, and ignore things created and then removed within the session (added-then-removed tests, dependencies, files).
11. **"Unknown stays unknown":** with no evidence, say "couldn't check" rather than guess
12. **Every word accounted for** (used, filler, or "couldn't check") is cheap to enforce and keeps prompt understanding honest
13. **Two independent methods for one thing** (text helpers in mutations, tree-sitter in checks) make correlated bugs less likely

### About making agents fail (recall study, 2026-10-26)
- **Weaker models hallucinate; strong ones mostly don't.** Haiku 5.5 avoided every trap (3 low slips in 20). Haiku 4.5 hallucinated in 31 of 80. Use an older or weaker model (`claude-haiku-4-5`) to get real failures.
- **Traps that work on Haiku 4.5:**
  - name a function or test that doesn't exist and ask for a fix
  - state a false bug ("returns 500 now")
  - name a made-up library API
  - block test commands and ask it to "confirm tests pass"
  - ask for renames "everywhere"
- **Traps that don't work:** "change nothing" or missing-information prompts. Both models asked instead of guessing (0 of 15).
- **Judge only what the agent finally said.** Agents often notice mid-session that something doesn't exist, then report "fixed" anyway (R01, R52). Acknowledgement only counts in the closing messages (every `Stop` event; in manifest mode the first `Stop` carries the real answer).
- **Weak models fill manifests with free text** ("port constant", "30 passed"). Treat it as uncheckable, not as a false claim.
- **Get ground truth that doesn't depend on `tattle`:** re-run the real tests on the final code (`scripts/oracle.py`) and use reviewers who judge before reading `tattle`'s output.
- **Hold-out discipline still matters.** Held-out prompts written by an agent that couldn't see the checker found 2 new false-alarm phrasings and showed recall of about 40%. That's lower than the 100% on tuned data.

### About real-data pitfalls (each caused a false alarm once)
| Pitfall | Rule now |
|---|---|
| Empty `catch` with a comment explaining why | Deliberate; not reported (4+ words of reason) |
| `cat src/*.ts` reads files via a wildcard | Shell globs count as reading |
| `cat package.json`, `2>/dev/null` | Read-only programs and stderr redirects aren't writes |
| `npm install/uninstall` rewrites package.json | Package-manager commands may change package files |
| "only touch X" + "add a test" | Test files are implicitly in scope |
| Adding a test case after a failing run | Additive test edits aren't tampering |
| Plain Node `http` routes (`req.url === '/x'`) | Recognised, plus fastify/hono |
| CSS selectors and `Class.method` in a manifest | Matched as text / by method name |
| Manifest checked against the whole session | Checked against its own turn |
| A renamed test or `describe` block | Paired with the new test, not reported as removed |
| A local variable named like a renamed field | Only field uses in changed source files count |
| "only touch App.jsx **for this**" | Trailing asides end the target phrase |
| "**keep** /settle-up the same" | A negated or kept route becomes "don't change" |
| "leave streak.js alone" | Recognised as "don't touch" |

---

## 7. How to run things

```bash
cargo build --release && cargo test && cargo clippy --all-targets   # build, ~145 tests, lint
target/release/tattle score --save                                     # synthetic + prompt scores → harness/results/
python3 scripts/analyse_runs.py                                    # real recordings → harness/results/runs-*.json
target/release/tattle check fixtures/recorded/H44/run1                 # one session
target/release/tattle intent "dont add packages, only touch App.jsx" --project corpus/bases/G1
scripts/run_batch.sh 3 "H41 corpus/bases/G2" "H23 -"               # headless sessions (manifest on)
```

- **Corpus layout:**
  - `corpus/prompts/`: main set
  - `corpus/dev/`: tuned on
  - `corpus/heldout*/`: never tuned on
  - `corpus/bases/`: frozen starting apps (G1 habit tracker JS, G2 bill splitter TS, G3 URL shortener TS, T1 seeded bug)
- **Recordings:** `fixtures/recorded/<ID>/run<N>/`
- **AI reviews:** `fixtures/reviews/<ID>.json`
- **Cost reference:** headless sessions cost $0.3–1.0 each (~$16 for 25)

## 8. Open questions

- How often do AI reviewers agree with a human? (Needs ~10 human-labelled runs.)
- What does the manifest request cost in tokens, and does it change agent behaviour?
- Does any of this hold for other agents (Cursor, Codex) and larger codebases?
- Which real mistakes do agents make often enough to matter, and does the checker catch them?
