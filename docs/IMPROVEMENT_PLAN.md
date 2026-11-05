# Making tattle as strong as possible without AI

**Status:** proposed (2026-10-27). #1 and #2 are built (2026-10-29; gate pending, see `docs/PHASE_4B_PLAN.md`). #4 is built without its optional execution part (`claims.flaky_unobserved`). The rest is not. It combines four research reports by agents that read the results, the code and the review files, and looked for prior art. All methods are deterministic: rules, static analysis, and running the user's own code in the sandbox. No model is involved.

## What's still missed

There are 49 sessions where reviewers found a hallucination. Grouped by type:

| Group | Sessions | Caught today |
|---|---|---|
| "Fixed"/"restored" something that never existed or never broke | ~17 (R01r2, R02r2, R04r2, R22, R26, R27, R52, R60, R63, R64, R81, R82, R85–R89) | R81; R63, R64, R02r2 only on data the checks were built on |
| Tests bent to match a behaviour change or a bug | R84, R98, R82, R97 | Partly, as "test edited after failure" |
| False capability or technique claims ("uses transactions", `upsert()`, "same behaviour") | ~11 | R72 and R109 (phantom names) |
| Wrong numbers, `file:line` references or counts | ~6 | none, all low severity |
| "Done" when features are missing (app starts, its own tests pass) | C1 (21 turns), parts of A1 and B | none |
| Wrong diagnoses, unmeasured "faster" claims, wrong arithmetic | R27, R55, R66, R90, R24 | none |

**Key point from the snapshots:** genuine fixes often come with no test at all (R81, R91, R93 are one-line source fixes). So "no fail-to-pass test" can't be a finding by itself. What separates a genuine fix from a fake one is whether behaviour changes on some input.

## Ranked plan

Ordered by expected gain per unit of effort. Each step is built test-first (mutations plus controls), then measured on a new blind batch.

| # | What | Catches | Effort | FP control |
|---|---|---|---|---|
| 1 | **P5 fail-to-pass accounting.** Run the final test files against the code from the start of the turn, getting per-test results from the vitest JSON reporter. If a test the agent cites as "now passes" or "verifies the fix" already passed on the old code, that's `claims.fix_not_demonstrated` | R64, R85, R86, R63; a cleaner `fix_unreproduced` | ~1 session | Fire only with a fix claim. Separate crashes from assertion failures (SWE-Factory "error2pass") |
| 2 | **P4: old tests on new code, joined to test edits.** Tests that passed at the start fail on the new code, and the turn edited those tests: `exec.tests_bent` | R84, R82 (R97 grey) | ~1 session | A finding only for fix requests (a genuine fix never breaks a passing test); info for feature or change requests |
| 3 | **Reproduce the symptom on the old code.** Send literals mined from the prompt (URLs, `[1,1,2]`, numbers, quoted strings) and from the tests to the matching routes and functions on the start snapshot. If the reported 500, wrong result or crash never appears and the agent says "fixed": `exec.symptom_not_reproduced` | R60, R82, R84, R85, R87, R26: about +3–5 | ~1–1.5 sessions | Only for bug-report prompts. A reproduced symptom whitelists the fix |
| 4 | **Flaky claims never observed.** The prompt or claim says flaky, no run ever disagreed, and the agent says "fixed". Optionally run the named test about 30 times on the old code: `claims.flaky_unobserved` | R22, R52, R89 | ~0.5 session | Divergent runs, or a reproduction, clear it |
| 5 | **HTTP differential sweep, old vs new.** Routes come from runtime enumeration (Express `app._router.stack`, Express 5 `app.router.stack`). Requests are seeded from the agent's own test traffic and boundary values, run as stateful sequences (a POST's id feeds `/:id`). A determinism shim pins random, time and ids; everything runs twice. Differences are reported in tiers, most certain first: a crash or 5xx introduced; any difference in a refactor-only turn; differences outside the prompt's scope; and in-scope differences (info only) | Silent breakage (R97 as info), A1-style partial breaks, refactors that change behaviour; positive evidence for genuine fixes | ~2–3 sessions | Tiers 1–3 are findings; tier 4 is info. In-scope changes are suppressed when tests or the closing message mention them |
| 6 | **Claim layer for closing messages** (`checks/claims.rs`: a small claim ontology, each claim resolved against evidence `tattle` already has) | | ~1.5 sessions | Unhedged sentences only. Each kind is resolved deterministically or becomes info |
| 6a | `claims.fixed_nonexistent`: "fixed/restored X" where X isn't in the start tree, and the diff is purely additive | R01r2, R02r2, R04r2 | | |
| 6b | `claims.identifier_ungrounded` plus a technique lexicon ("uses transactions", "parameterised", "cached", "indexed") that needs a code marker | R14, R72, R109 (R26) | | |
| 6c | Number pack: "N existing + M new" vs real counts; "all N pass" and per-file counts vs the execution record; `file:line` vs the code; "didn't change X" vs the diff | R13, R15, R01r2, R31 (low severity) | | Execution counts only (static counts break on `it.each`) |
| 7 | **Probes from spec-style prompt lines.** A small grammar for `METHOD /path … -> STATUS`, "N to M characters", "unknown X → 404", "one X or 404", turned into sandbox probes (route exists, status mapping, boundary M passes / M+1 → 400, 401 without a token, response keys). Fixtures come from the route graph (POST → id) | Missing routes and wrong status codes behind "done" claims (C1, B runs, A1 milestones) | ~2–3 sessions | Report only when setup succeeded, else "couldn't probe" |
| 8 | **Executable manifest examples.** The manifest's requirements carry examples (HTTP steps, CLI argv, function calls with expected results) that `tattle` runs on the old and new code. Rules against gaming: each example quotes the prompt verbatim; its literals must appear in that quote; it must fail on the old code for add/change requests; flipped expectations must fail (example mutation); prompt literals with no example are reported | "Done" claims on any prompt where the agent writes examples | ~2 sessions plus a hook change | Malformed examples are uncheckable, not false. Needs measuring for cooperation on Haiku 4.5 |
| 9 | **Static route and status diff.** Use `RepoIndex` routes plus status literals before and after, compared with prompt-derived routes: `intent.status_not_returned` | Missing routes or statuses when execution is off | ~0.5 session | Static fallback for #7 |
| 10 | **Function and CLI differentials** for libraries and CLIs. Type-guided boundary inputs plus literals mined from the function body, for changed exported functions; argv sets from help text and option definitions | B3, B8 libraries; B2, B7 CLIs | ~2 sessions each | Same tiers as #5 |

**Also cheap:** a revert-one-hunk tag. Revert each changed source hunk on its own and re-run the suite: "pinned by tests" or not. It explains no-test fixes in reports.

## Expected effect

The researchers' estimates, untested:
- **#1–#4** take the fake-fix group from 1 of 7 caught (blind) to about 5–7 of 9, with no flags on the genuine fixes R91–R95. Reproduced symptoms and observed behaviour changes keep them clear.
- **#6** adds about 6–8 low-to-medium-severity sessions.
- **#7 and #8** target false "done" claims, the C1 pattern. #7 needs no agent cooperation, but only works on spec-like prompts. #8 works on any prompt, but depends on the agent writing usable examples.
- **Overall:** from about 40% of hallucinating sessions caught (blind, static only) to perhaps **65–75%**. That still has to be measured on a new blind batch.

## What stays out of reach without a model

- **Wrong diagnoses** (R27, R66, R98's explanation)
- **Unmeasured "faster" claims**
- **Wrong arithmetic in prose** (R24)
- **Claims about algorithms** ("greedy is optimal", R90)
- **Behaviour that nothing (prompt, tests or manifest) pins down**

These can be narrowed, for example by benchmarking when the prompt asks for speed, but not closed.

## Build order

1. Finish 4b.3 (app start).
2. Build #1 and #2 together: one more run per snapshot.
3. Then #4, then #3.
4. Measure on a blind batch.
5. Then #6.
6. Then #5, with #9.
7. Then #7, then #8 (it needs a hook change and a cooperation study).
8. Then #10.

Each step keeps the hard rules: no AI, deterministic reports (findings only from saved records), and 0 false alarms on all labelled sessions.

## Sources

The four research reports (2026-10-27) cite:
- the SWE-bench fail-to-pass definition and SWE-bench-Live
- SWE-Factory, on error2pass
- DiffTGen / Opad and the patch-assessment survey (arXiv 2301.01113)
- BERT and Randoop-style differential regression testing
- Schemathesis, RESTler and EvoMaster, on stateful API fuzzing
- iComment, @TCOMMENT and FRACO, on rule-based comment/code consistency
- StrykerJS
- EARS, for controlled requirement grammars
