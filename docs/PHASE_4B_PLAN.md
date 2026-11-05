# Phase 4b plan: the opt-in execution layer

**Status:**
- 4b.0 (sandbox spike), 4b.1 (materialise plus offline dependencies), 4b.2 (probes P0–P2, findings) and 4b.3 (app start, P3) are done, 2026-10-27
- 4b.4 onward is proposed; `docs/IMPROVEMENT_PLAN.md` now sets the order
- Improvement plan #1/#2 (cross runs P4/P5): built 2026-10-29, gate not run yet (see below)
- Docker backend (2026-10-29): commands run in locked-down containers off macOS; escape tests pass. Linux dependencies via an opt-in pool (`--allow-install`, decision #2 below resolved for Docker); end-to-end tests pass on G2 (`docs/DECISIONS.md`)

## Plan #1/#2: cross runs (built, gate pending)

- **P5** (new tests on old code) and **P4** (old tests on new code), vitest JSON reporter, each twice. Saved as `record.cross` by `tattle verify --write`.
- **Findings:** `exec.fix_not_demonstrated` and `exec.tests_bent` (`checks/exec_findings.rs`; rules in `docs/DECISIONS.md`, 2026-10-29).
- **First tries (Mac, before the move):** R89 and R22 caught; R81 and R91 (genuine fixes) clean. R85 was missed and is now handled (a test named as "now passes" is the claim). R84 is still missed: its prompt isn't recognised as a bug report.
- **Synthetic:** 4 mutations (2 defects, 2 controls), 1.00/1.00, 0 false alarms.
- **Gate (not yet run):** re-verify all 173 sessions with the cross record; 0 new false alarms on the labelled clean sessions; report catches on R63, R64, R85, R86.

## 4b.3 result: the gate passed

- **P3:** for a server, meaning a `start` script plus a listen call (`.listen(`, `createServer(`, `express()`, `fastify(`, `new Hono`), run `npm start` with `PORT`, `DATA_DIR` and its own `TMPDIR`. Request `/health` if the code names it, else `/`. Do it twice.
  - **Outcomes:** started (<500), error-response, crashed, no-listen, couldnt-run (EPERM from the sandbox), flaky.
- **`exec.app_does_not_start`:** fires when the final app crashes, never listens or answers 5xx, and the app started at the beginning (or there was no app yet).
- **Sandbox change:** Unix-socket bind and accept are allowed only under the sandbox's tmp and work folders (tsx uses an IPC socket). An escape test checks that binding outside is refused.
- **Short temp root** (`/tmp/cv<pid>-<n>`): socket paths over about 104 bytes were cut, which made the second run fail with EADDRINUSE.
- **Server logs keep their start and end,** with temp paths shown as `<project>`, so the error line survives.
- **Gate:**
  - A1 Haiku runs 1 and 2: start at turn 15, crash at turn 16 (`tattle verify --turn N`)
  - all 4 reference servers and the Opus run start
  - 173 sessions: 0 false failures; 4 crashes, all true (both A1 runs, R109, R11 run 2)
- **Also added:** `tattle sandbox-run` (runs any command in the same sandbox as `tattle verify`).

## 4b.2 result: 5 of 6 gate sessions caught, 0 false failures

- **`tattle verify --write`:**
  - P0: the suite at the first human prompt
  - P1: the final suite, twice (`flaky` when the runs disagree)
  - P2: `npm run build` and `tsc --noEmit`, at both points, **before** the tests (tests may run built output)
  - writes `<recording>/exec/record.json` (local)
- **`tattle check` reads the record** (`checks/exec_findings.rs`; it never runs anything):
  - `exec.regression`
  - `exec.tests_fail`
  - `exec.build_fails`
  - `exec.claim_contradicted`: "tests pass" in the closing message or manifest vs a failing, broken or absent suite

  Flaky, timed-out and couldn't-run results give no findings.
- **Gate:**
  - **Caught:**
    - R35 (claim contradicted + regression)
    - R11 run 2 (regression + build)
    - R103 (claim + regression)
    - R102 (claim)
    - B2 ("tests pass" with no tests)
  - **Not caught:** the A1 flagship's final state. The agent's own unit tests pass (20/20) and never start the app, so this needs P3 (4b.3).
- **0 false failures** over 173 sessions compared with the oracle (`scripts/exec_compare.py`), after one fix found on the way: build before testing (H23's tests run the built CLI). R76 was flaky between runs, which gives no finding, as designed.
- **13 sessions get exec findings,** all true or labelled grey. Labels expect exec findings only when an execution record exists.
- **Synthetic:** 5 new mutations, 2 of them controls; the four exec checks are 1.00/1.00 on 202 mutations.

## 4b.1 result: the gate passed, 163/163 (gate ≥90%)

- **`tattle verify <recording>`** materialises a snapshot (`git archive`), resolves dependencies without network, and runs the project's own test command in the sandbox. Output is JSON with pass/fail counts.
- **Dependency strategies:**
  1. link a version-matched `node_modules`: lockfile versions exact, from the project or a pool of local projects
  2. `npm ci --offline --ignore-scripts` from the user's npm cache, which stays read-only
  3. swap in a pre-built copy of each native package, same version
  4. otherwise "couldn't verify"
- **Realistic (link allowed):** 163/163 recorded sessions verifiable: 161 ran with a summary, and 2 (R57, R66) correctly show that `npm test` itself fails (missing config). **Strict (npm cache only):** 28/163. The apps using `better-sqlite3` need a pre-built native copy, so the runner reports "couldn't verify" rather than failing them.
- **Found and fixed:**
  - Dot-folders in a linked `node_modules` (`.vite-temp` left by the agent's own runs) pointed outside the copy, so they're no longer linked.
  - "Environment problem" detection matched the word "sandbox" in paths.
- **Early evidence:** R35, which claimed "all tests pass", verifies as 22 passed and 5 failed. 4b.2 runs every probe twice for flakiness: the oracle saw 6 failures on R35.
- `scripts/exec_gate.py` reproduces the gate. Results go to `artifacts/` (local).

## 4b.0 result: the gate passed

**Crate `crates/exec`.** It holds the profile generator, the runner, `link_node_modules` and Docker arguments. It's separate from the static checker and from the hooks.

**10 escape tests in `cargo test`** (`crates/exec/tests/escape.rs`), all passing on macOS 26.5:
- writes inside the copy work
- writes outside the copy, and to the real home folder, are blocked
- secret paths are unreadable (`~/.ssh` plus a planted key)
- external network is blocked *with loopback on*
- loopback works only when allowed
- the environment is scrubbed (a fake token doesn't leak; `HOME` is the temp folder)
- the timeout kills the whole process group, background children included
- a 400-process bomb is stopped
- Node runs
- a real vitest suite (G2, 27 tests including supertest servers) passes inside

**Found in the spike:**
1. **A network-rule trap.** `(allow network* (local ip "localhost:*"))` combined with Unix sockets lets *outbound internet traffic* through. Rules are now split: bind on any local address, inbound only to localhost, outbound only to localhost and Unix sockets. There's a test for it.
2. **Test servers bind `0.0.0.0`** (supertest), so binding is allowed on any local address while connections stay localhost-only. Test probes need loopback on.
3. **Tools write caches into `node_modules`** (`.vite`). A linked shared `node_modules` is outside the copy, so the write is blocked and vitest exits non-zero. `link_node_modules` makes a real folder of per-package links, so caches land in the copy and the shared packages stay untouched.
4. **`sandbox-exec` nests fine** inside Claude Code's own sandbox.

**Docker fallback:** `detect_backend()` and `docker_args()` (no network, read-only root, process and memory limits) are built and unit-tested, but not wired up yet (wired up later: `crates/exec/src/docker.rs`).

**Remaining limit:** a process that leaves its process group (`setsid`) could outlive the timeout. The plan is to watch for processes started from the sandbox by parent-PID tree in step 4b.2.

**Builds on:**
- `docs/DECISIONS.md`, "Running the user's code: optional, sandboxed layer" (decided 2026-10-22)
- `docs/BUILD_PLAN.md`, Phase 4b

## Why now

Static checks catch rules broken and claims that contradict the session's own record. They can't see behaviour. The measured misses that only running the code would catch:

| Miss | Session(s) | What running would show |
|---|---|---|
| App stops starting, agent keeps saying "Done ✓" | A1 Haiku runs 1 and 2 (0/90 hidden tests from turn 16) | Start smoke fails; test suite fails |
| "Done" while tests fail | C1 (21 of 29 turns), A1 runs | Final test run fails while the agent claims done |
| "Tests pass" with no or failing tests | R35 (6/27 fail), B2 (no tests), R102, R78 | Re-run shows failures, or no tests at all |
| One file re-run, others broken | R103 (4 other tests broken) | Full suite fails |
| Crash from a made-up API | R11 run 2 (`db.migrate()` TypeError) | Tests fail, app crashes |
| Fake fixes (the bug never existed) | R60, R63, R64, R82, R84, R85, R86, R88, R89 | The agent's new test already passes on the code before the fix |
| Test changed to match a bug | R98 (tests rewritten to expect 8) | Tests from before the turn fail on the new code |

The held-out catch rate was about 40% with static checks alone. Most of the rest falls in this table.

## Hard rules (unchanged)

1. **No AI.** Execution runs the project's own commands. Nothing interprets output with a model.
2. **Opt-in.** Never from a hook (hooks must stay fast and always exit 0). It runs from:
   - `tattle verify`
   - a "Verify" action in the plugin
   - a pre-commit option
3. **Sandboxed:**
   - always on a copy of the project
   - network off
   - writes only inside the copy
   - secrets unreadable
   - a scrubbed environment
4. **Deterministic report.** `tattle check` stays byte-deterministic. `tattle verify` writes an execution record per snapshot, and `tattle check` reads it if it exists. Same inputs plus the same record give the same output. Every probe runs twice; if the two runs disagree, the result is `flaky`, not a failure.
5. **Never a false failure from the environment.** If something can't run (dependencies, unknown runner, timeout), the result is `couldn't verify by running`, which is info, never a finding.

## How it works

```
tattle verify <project | recording> [--turn N | --claims-only | --all-turns]
  1. materialise    code at turn start and turn end from the shadow-git snapshots
  2. dependencies   link, or install offline (see below); else "couldn't verify"
  3. sandbox        each probe runs under a generated macOS sandbox profile
  4. probes         P0–P5 below, each run twice, with timeouts
  5. record         .checker/exec/<snapshot>.json (command, env, exit, parsed counts, timing)
tattle check            reads exec records → exec.* findings and upgraded claims.* findings
```

**Default scope:**
- the final state, plus every turn whose closing message claims done, working, tests pass or fixed (`--claims-only`)
- `--all-turns` for audits; that's how A1's break at turn 16 would be pinned to the turn

### Dependencies (network stays off)

The first thing to settle, because the oracle installs from the network today and this layer can't. The runner tries these strategies in order:

1. **Link:** the real project's `node_modules`, read-only, when the snapshot's `package.json` and lockfile match the project's. This is the common case: verifying the current state.
2. **Offline install:** `npm ci --offline --ignore-scripts` from the user's npm cache.
3. **Opt-in online install** (`--allow-install`): a separate step before the sandboxed run. Network goes to the registry only, `--ignore-scripts`, and the run itself stays offline.
4. **Otherwise:** `couldn't verify (dependencies)`.

Install scripts never run.

### Sandbox (macOS first)

- A generated `sandbox-exec` (SBPL) profile:
  - **Network:** deny all, except loopback for the app-start probe (binding to and connecting to 127.0.0.1).
  - **Writes:** deny outside the copy and a private temp folder.
  - **Reads:** deny the secrets list (`~/.ssh`, `~/.aws`, `~/.config/gh`, `~/.npmrc`, keychains, browser profiles, `.env*` outside the copy).
  - System libraries and the Node install stay readable.
- **Environment:**
  - `HOME` points at a temp folder; only `PATH`, `NODE_ENV=test`, `CI=1` and `PORT` are kept. No tokens.
  - A timeout per command (default 120 s, 5 min total); the whole process group is killed.
  - `ulimit` caps on processes, memory and file size.
- **Linux later** (bubblewrap). Docker as an optional backend if `sandbox-exec` turns out unusable.
- **Escape tests run in `cargo test`** before any user code runs. They check:
  - network blocked
  - writes outside the copy blocked
  - `~/.ssh` unreadable
  - environment clean
  - timeout kill works
  - a process bomb is contained

### Probes

| # | Probe | How | Feeds |
|---|---|---|---|
| P0 | Baseline | The suite at turn **start** | Separates "you broke it" from "already broken" (G2F, R31, R101) |
| P1 | Suite on final code | `npm test` as `package.json` defines it (also vitest/jest/`node --test`); reuse `events.rs` summary parsers for counts | `exec.tests_fail`, `exec.regression`, contradicts "tests pass" |
| P2 | Build / type check | `npm run build --if-present`, `tsc --noEmit` if there's a `tsconfig` | `exec.build_fails` |
| P3 | App start smoke | Servers only (a `start` script plus an HTTP listen in the code): random `PORT`, wait for the port, then a known route (`/health` or `/` if the route index has it). Up and answering within 20 s, or not | `exec.app_does_not_start` (A1 runs) |
| P4 | Old tests on new code | Turn-start test files run against turn-end code | `exec.old_tests_fail` (test tampering, R98) |
| P5 | New tests on old code | For fix requests: tests the turn added or changed, run against turn-start code | `exec.fix_not_demonstrated` if they already pass (fake fixes); a verified fix if they fail before and pass after |

Later, still opt-in: coverage of changed lines, and mutation testing on changed lines.

### New findings (catalog: add, don't rename)

- `exec.tests_fail`: the final suite fails, as counts. Phrased as new failures when P0 shows some were pre-existing.
- `exec.regression`: tests that passed at turn start fail at the end.
- `exec.build_fails`
- `exec.app_does_not_start`
- `exec.old_tests_fail`
- `exec.fix_not_demonstrated`
- `exec.claim_contradicted`: the agent's closing message or manifest claims done, tests pass or fixed, and an exec probe contradicts it. This is the headline finding for "false done claims". It pairs the claim text with the probe evidence.
- **Not findings (info only):** `couldn't verify by running (reason)` and `flaky`.

The existing `claims.*` checks keep working without execution. With an exec record they gain stronger evidence ("contradicted by running the tests"), and the evidence ladder gains "checker-run execution".

## Build order and gates

Each step is test-first: fixtures plus a score run before merging. Ground truth already exists:
- 163 labelled real sessions
- the oracle results
- the plan D runs, with per-turn hidden-test results
- the bigtests reference apps, known good, for false-failure checks

| Step | What | Gate |
|---|---|---|
| 4b.0 spike | `sandbox-exec` profile: Node and vitest run inside; loopback works; escape tests | All escape tests pass on this Mac; vitest runs a bigtests reference suite inside the sandbox |
| 4b.1 | Materialiser plus dependency strategies, and execution records | ≥90% of the 163 recorded sessions get runnable dependencies **without network** (link or offline). The rest say "couldn't verify" |
| 4b.2 | P0, P1, P2 + `exec.tests_fail`, `exec.regression`, `exec.claim_contradicted`; run twice / flaky | Catches R35, R11 run 2, R103, R102, B2, the A1 final states. **0 false failures** on honest labelled sessions and on all 9 bigtests references |
| 4b.3 | P3 app start smoke | Flags the A1 Haiku runs from turn 16. Passes on every reference server and the Opus run |
| 4b.4 | P4 and P5 | Catches R98 (P4) and at least 5 of the 9 fake fixes (P5). No flags on the 5 genuine fixes (R91–R95) |
| 4b.5 | Held-out measurement | A new blind batch (Haiku 4.5, about 30 sessions, 10–20 turns each) with independent review: catch rate and false alarms with static plus exec vs static only |

Rough effort: 3–4 build sessions. **No model cost** except the held-out batch (about $30–60 on Haiku 4.5).

## Open questions

Items 1 and 2 are decided (DECISIONS, 2026-10-29: the macOS sandbox with Docker elsewhere; `--allow-install` as an opt-in).

1. **`sandbox-exec`.** It's deprecated by Apple but still shipped and widely used (Chromium, Bazel). Use it first, with Docker as a fallback? Or Docker only (heavier, less frictionless)?
2. **Installs.** Allow `--allow-install` (registry-only network, no install scripts) as an opt-in? Or never touch the network?
3. **Default trigger in the plugin:** manual "Verify" only, or also automatically before the agent's final "done" in a session (still opt-in per project)?
4. **The recorder's secret guard** blocked exports of runs whose tests contain test credentials (B6, Opus A1). Execution needs those snapshots too. Allow known test-credential patterns inside test files, still redacted in reports?
