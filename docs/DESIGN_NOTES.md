# Design Notes

A record of the design discussion that produced `BUILD_PLAN.md`. The latest decisions are in `DECISIONS.md`.

---

## 1. The problem

Build a tool that checks AI coding agent output against what the human asked for.

**Requirements**
- Static code: no agents, no model calls in the analysis
- Secure and local only
- Accurate, tested, fast enough to run on every turn
- Produces output a human or an agent can read and act on (it doesn't write code)
- Works alongside existing coding agents (Claude Code, Cursor, …)
- **Frictionless**: the human never writes out instructions or specs
- Must handle greenfield code, not just diffs

**Core tension:** turning a prompt into checkable instructions needs interpretation, and interpretation isn't static.

---

## 2. Ways around the interpretation problem

Ranked by value:

1. **Agent-declared intent manifest.** The coding agent already interprets the prompt, so have it write a structured manifest (files touched, functions added, constraints, tests). The checker verifies each claim against the code statically. The agent's words are claims, never evidence.
2. **Prompt-independent checks** (probably most of the value): test tampering, stubs, hallucinated imports, scope creep, swallowed errors, security regressions, convention drift. These need no prompt at all.
3. **Literal prompt extraction:** only what's mechanically in the prompt (code spans, paths, identifiers, a fixed constraint-phrase lexicon). Honest coverage: verified / violated / couldn't check.
4. **Local model as a compiler step (optional, off by default):** pinned weights, temperature 0, cached, output editable. Only the spec-building step is learned; the analysis stays static.
5. **Standing project rules:** written once or mined from the codebase (layering, forbidden imports, conventions).

---

## 3. Low-code user workflow

- **Install:** one thing (Claude Code plugin, extension or signed app). Auto-detects agents; one click per agent writes hooks and rules and shows what it changed.
- **Presets instead of rules:** Prototype / Real users / Money or personal data.
- **First-run scan:** "here's what I learned about your project" → confirm.
- **Per turn:** silent snapshot → agent works → checks run at `Stop` → fixable issues sent back to the agent automatically (max 1–2 rounds) → human sees one plain-English summary card.
- **Card:** ✅ / ⚠️ / ❔ lines with reasons, plus `[Ask AI to fix]` `[Undo this change]` `[That's fine]`.
- **Undo is the headline feature** for low-code users.
- Silent when everything passes. "That's fine" teaches the tool what to stop flagging.
- Browser-only builders (Lovable, Bolt, v0, Replit) are out of scope for v1.

---

## 4. Capture layer

Three things to capture: **the prompt**, **what the agent did**, and **code before and after**.

| Source | Mechanism |
|---|---|
| Claude Code | `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `SessionStart` hooks; transcript JSONL path passed to the hooks |
| Cursor | Its hooks system (fewer details) |
| Codex / others | Read their local session logs through an adapter |
| Fallback | File watcher + "Check now", which runs the prompt-independent checks only |

**Snapshots:** a shadow git repo at `.checker/shadow.git`, separate from the user's git history. Committed at each prompt submit and at `Stop`.

**Normalised event log:** one adapter per agent → one shared schema:

```json
{"turn": 7, "seq": 12, "type": "prompt",  "text": "..."}
{"turn": 7, "seq": 13, "type": "read",    "path": "src/routes/auth.ts"}
{"turn": 7, "seq": 14, "type": "edit",    "path": "src/routes/auth.ts"}
{"turn": 7, "seq": 15, "type": "command", "cmd": "npm test", "exit": 1}
{"turn": 7, "seq": 16, "type": "final",   "text": "Done! All tests pass"}
```

---

## 5. Three fact sources

**A. Code facts** (before/after snapshots via tree-sitter): symbols, imports, dependencies, test and assertion counts, stubs, swallowed errors.

**B. Behaviour facts** (event log, no language handling needed):

| Fact | Check |
|---|---|
| Edited a file it never read | Changed code it hadn't looked at |
| Last test run before the last edit | Tests not re-run |
| Failing run → test file edited → passing run | Possibly changed the test to make it pass |
| "Tests pass" in final message, last exit ≠ 0 | False claim |
| `--no-verify`, `rm -rf`, force push, installs | Risky command |
| Edits outside the project | Out of bounds |

**C. Intent facts** (prompt text). Key idea: **grounding**. Words in the prompt become targets only if they match something real in the repo index (files, symbols, routes, packages, tests).

---

## 6. Normalising a prompt into intent facts

**Fact format:** closed list of kinds, versioned, with a source span on every fact.

```json
{
  "id": "f12",
  "kind": "constraint.deps_unchanged",
  "polarity": "must",
  "target": null,
  "params": {},
  "source": { "turn": 7, "span": [52, 74], "text": "Don't add new packages" },
  "grounding": "lexicon:v3/rule:no-deps-01",
  "confidence": "exact"
}
```

- Kinds (initial): `target.modify`, `target.unchanged`, `scope.only`, `constraint.deps_unchanged`, `constraint.tests_pass`, `require.tests_added`, `param.literal`
- Polarity: `must` / `must_not` / `prefer` (soft constraints such as "if you can avoid it")
- Confidence levels: `exact`, `single-match`, `multi-match`, `inferred` (no percentages)

**Pipeline (deterministic):**
1. Protect code spans, paths and identifiers
2. Split into clauses
3. Match the constraint lexicon (polarity words + object words)
4. Ground nouns against the repo index
5. Extract numbers with units, quoted strings
6. Merge turns (last write wins per target and kind, with the replacement recorded)
7. Account for leftovers

---

## 7. Integrity rules

**Rule 1: every part of the prompt is accounted for.**
consumed + filler + residual = the whole prompt. Residual is shown verbatim as "couldn't check". Enforced in code and tests. Collected residuals show which lexicon rules to add next.

**Rule 2: every code change is explained.** Reconcile the snapshot diff against edit events:

| Diff | Events | Result |
|---|---|---|
| Changed | Edit event | Explained |
| Changed | Shell command only | Changed by a command |
| Changed | Nothing | Unexplained change |
| Unchanged | Edit event | Reverted / no-op |

**Rule 3: the log itself is complete.** Sequence numbers, the hook stream compared with the transcript at `Stop`, the prompt saved at submit time (transcripts may be compacted), and **fail closed**: "Partial check" instead of a green tick.

**Leak prevention (data):**
- Redact secrets on capture, before writing to disk
- Auto-add `.checker/` to `.gitignore` (the most likely real-world leak)
- Exclude `.env*`, key files and gitignored files from snapshots
- Logs readable only by the user, with old turns deleted after a set time
- Facts hold references, never secret values (the report goes back to the agent)
- No network access, enforced rather than promised

**Leak prevention (meaning):** covered by Rule 1.

---

## 8. Code as objective truth: the evidence ladder

| Level | Source | Can confirm |
|---|---|---|
| 1 | Code state (hashed snapshot) | Everything below |
| 2 | Recorded execution (exit codes captured by the agent tool) | 3–5 |
| 3 | Recorded actions (tool-call log) | 4–5 |
| 4 | Agent statements (final message, manifest) | Nothing (always a claim) |
| 5 | Prompt interpretation (intent facts) | Nothing (always a requirement) |

Each check is a requirement or claim (levels 4–5) tested against evidence (levels 1–3).

- Every fact is tagged with its snapshot hash. Output must be deterministic, and that's tested.
- **Structural facts** (exists, added, deleted) are high certainty. **Behavioural facts** (is it actually wired up?) come from static reachability where provable; otherwise the result is "defined but couldn't confirm it's wired up".
- The checker never runs code. Behaviour evidence beyond static analysis comes only from level-2 recorded test runs.

---

## 9. Pipeline overview

```
Prompt text ──► clause split ──► lexicon + repo grounding ──► intent facts  ─┐
                       └──────────────► residual (shown verbatim)           │
Event log  ──► adapter ──► action / exec / claim facts ────────────────────┤
Snapshots  ──► tree-sitter / index ──► code facts (hashed) ────────────────┤
                                                                           ▼
                            reconcile (diff vs events, sequence gaps)
                                                                           ▼
                            check templates: requirement/claim vs evidence
                                                                           ▼
                     ✅ verified · ⚠️ violated · ❔ couldn't check · ⛔ capture incomplete
```

Output: Markdown (human) + JSON/SARIF (agents and tools). Each finding carries evidence (file:line, event number), its evidence level and a confidence level.

---

## 10. Known limits

- Vague prompts ("make it nicer") will always be "couldn't check". That's correct behaviour.
- The constraint lexicon needs the most testing. English only at first.
- Fuzzy grounding can be ambiguous → report low confidence, don't guess.
- Coverage depends on what each agent exposes. Claude Code exposes the most.
