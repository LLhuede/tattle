# Big tests: from-scratch builds at scale

Four suites that test `tattle` on long, from-scratch builds instead of small
edits. Everything here is **reproducible test definition** and lives in git:
specs, turn scripts, hidden acceptance tests, persona rules, run plans and
reference implementations (used only to prove the hidden tests are
passable). **Nothing a run produces goes into git**: recordings, oracle
results and logs go to `artifacts/` (gitignored).

**Status (2026-10-27): plan D ran in full ($228); results in `docs/RESULTS.md` §3g.** Before that it was built and validated:
- `bigtest.py validate --with-reference`: 9 suites, 1 backlog and 1 plan, no problems.
- Every reference passes all its hidden tests (423 in total).
- The runner, the simulated-user loop and plan gates were tested end to end with `scripts/fake_claude.py`, a free stand-in for `claude`.
- Estimated cost of plan D: about $90–130 (cap $250).

| Suite | Milestones | Hidden tests | Turns | Traps |
|---|---|---|---|---|
| A-flagship (Splitwise-lite: auth, groups, expenses, settle-up, feed, UI) | 8 | 90 | 21 | 5 traps, 3 constraints, 2 requirement changes |
| b1-notes-api | 4 | 40 | 6 | false bug report, explain-only |
| b2-csv-cli | 4 | 42 | 6 | requirement change, confirm-tests (commands blocked) |
| b3-money-lib | 4 | 47 | 6 | non-existent helper, requirement change |
| b4-job-worker | 3 | 31 | 5 | requirement change, false bug report |
| b5-quiz-web | 4 | 41 | 5 | non-existent helper, false bug report |
| b6-inventory-api | 4 | 37 | 6 | confirm-tests (commands blocked), explain-only |
| b7-log-cli | 4 | 48 | 5 | non-existent dependency, requirement change |
| b8-cache-lib | 4 | 47 | 6 | false bug report, requirement change |
| C-simuser (A's product) | 9 items | A's 90 | ≤30 | false bug report, requirement change, confirm-tests, self-report |

| Suite | What | Runs |
|---|---|---|
| `A-flagship/` | One full-stack product built from a spec over ~20 scripted turns, with requirement changes, constraints set early and tested late, and planted traps | `bigtest.py run` |
| `B-breadth/` | 8 medium builds (3–6 turns each) of different shapes: API, CLI, library, browser app, worker | `bigtest.py run` |
| `C-simuser/` | A's product driven by a rule-based simulated user: runs the hidden tests after each turn, pushes back on "done" claims that fail, changes requirements | `bigtest.py simuser` |
| `D-hybrid/` | A plan that combines A, B and C runs with models, run counts, budget and stop gates | `bigtest.py plan` |

## Ground truth

Each suite has **hidden acceptance tests** (`acceptance/<milestone>.*.test.mjs`):
black-box tests through HTTP, the CLI or a named module, written against
the spec only (`CONTRACT.md`). After every turn the runner copies the agent's
code, installs and builds it, and runs the tests for the milestones due by
then (`scripts/acceptance.py`). That gives, per turn, what really works,
independent of the agent's own tests and of `tattle`. Reviewers then only need
to read the turns where the agent's claims and the hidden tests disagree.

Each suite's `reference/` is a minimal implementation that passes its hidden
tests, which shows the tests are correct and passable. Agents under test
never see this repository.

## Formats

### `suite.json` (A and each B app)
```json
{
  "schema": 1, "id": "A1", "title": "…", "kind": "server|cli|library|browser|worker",
  "spec": "spec.md",
  "base": null,
  "milestones": [{ "id": "m1", "title": "…" }],
  "budget": { "max_usd": 30, "max_turns_per_prompt": 80 },
  "turns": [
    {
      "n": 1,
      "prompt": "…{{spec}}…",
      "milestones_due": ["m1"],
      "deny_tools": [],
      "constraint": null,
      "trap": null
    }
  ]
}
```
- `{{spec}}` in a prompt is replaced by `spec.md`.
- `milestones_due`: milestones that should work after this turn. They're cumulative, so list only the new ones.
- `constraint`: a rule set in this turn that later turns must keep (for example "no new dependencies from now on"). Informational, for reviewers.
- `trap`: `{ "kind", "honest_if", "hallucination_if" }`, as in `corpus/recall/`.
- `deny_tools`: entries removed from the allow-list for this turn, for example `["Bash(npm:*)"]`.

### `backlog.json` (C)
```json
{
  "schema": 1, "id": "C1", "suite": "../A-flagship", "opening": "…{{spec}}…",
  "items": [{ "id": "…", "prompt": "…", "milestones": ["m2"],
              "symptoms": { "<test-name substring>": "what a user would say" } }],
  "events": [{ "after_item": "…", "kind": "requirement-change|false-premise|confirm-tests|self-report",
               "prompt": "…", "trap": null }],
  "policy": { "max_turns": 30, "max_pushbacks_per_item": 2,
              "done_claim": "<regex>", "pushback": "… {symptom} …", "generic_symptom": "…" },
  "budget": { "max_usd": 60 }
}
```
The simulated user is rules, not a model. It never quotes hidden test names. It turns a failing test into the user-level complaint in `symptoms` (or `generic_symptom`). `--llm-user` (off by default) lets a model reword the messages. That's harness-only, never part of `tattle`.

### `plan.json` (D)
```json
{
  "schema": 1, "id": "D1",
  "runs": [{ "suite": "bigtests/A-flagship/suite.json", "mode": "run|simuser", "model": "claude-haiku-4-5", "runs": 2 }],
  "order": "cheap-first",
  "gates": [{ "after": "<run label>", "stop_if": "<condition>" }],
  "budget": { "max_usd": 200 }
}
```

## Running (not done yet)
```bash
python3 scripts/bigtest.py validate                       # check every definition, no agents
python3 scripts/bigtest.py dry-run bigtests/D-hybrid/plan.json   # cost estimate + commands
python3 scripts/bigtest.py run bigtests/B-breadth/b1-notes-api/suite.json --model claude-haiku-4-5 --run 1
python3 scripts/bigtest.py simuser bigtests/C-simuser/backlog.json --model claude-haiku-4-5 --run 1
python3 scripts/bigtest.py plan bigtests/D-hybrid/plan.json     # runs the whole plan, gate by gate
```
- **Output:** `artifacts/bigtests/<id>/<model>/run<N>/`, which holds:
  - `turnK.json` (the agent's reply, cost, acceptance results)
  - `recording/` (the `tattle` export)
  - `summary.json`
  - `state.json` (for `--resume`)
- **Sandboxes:** `~/Code/cv-sandbox/big/`.
- **Stops:** a run stops at its budget or when the usage limit is hit. Restart it with `--resume`.
- **Recordings bigger than 500 KB** are allowed: the export just reports it.
