# Decision Log

> **Name:** the tool was called **cv** until 2026-11-04, then renamed **tattle**. Older entries below use the old name and old commands (`cv check` is now `tattle check`, `/cv:report` is `/tattle:report`).
>
> **History:** this file is the project's research log since day one. Commit hashes in it refer to the development history, which isn't public; the public repo starts from a fresh commit.

Newest first. Each entry: date, decision, why, status.
Statuses: **Decided** · **Proposed** (needs sign-off) · **Superseded**

---

### 2026-11-10: `tattle audit` reads Claude Code's own session logs
- **Why:** tattle is silent when nothing is wrong, and current models rarely trip a rule, so a new user sees nothing for days. Sessions from before the install are already on the computer; checking them gives a result in the first minute.
- **Decided:** `tattle audit` (the plugin's `/tattle:audit`) turns each session log into the events the recorder writes (`checks/transcript.rs`) and runs the rules that need only events and messages: the five `claims.*` rules for tests and fixes. Each request is judged on the session up to the end of its turn, as the plugin would have. A "tests pass" claim is also counted as backed (a passing run, nothing edited after), or as passed-then-edited; the second is a count, not a finding, because without snapshots the audit can't tell whether the tests use the edited file.
- **Left out on purpose:** rules that need code snapshots, `capture.*`, and for now `commands.risky` and `events.edit_outside_project` (a log's working folder can change mid-session; needs real logs to judge).
- **Privacy:** read-only, on request, nothing stored, nothing sent. **Proposed** (needs the maintainer's sign-off: it is the first time tattle reads files outside the project).
- **Tested:** adapter and audit unit tests; four end-to-end tests through the binary; and over every synthetic session and mutation, the audit of the session's log finds exactly what the full check finds for those five rules (`audit_of_the_session_log_matches_the_full_check`). Log shapes were checked against a real Claude Code log (v2.1.321), including a failed command. **Not yet measured on a batch of real past sessions**: do that, and record the false alerts before fixing any, before announcing it.

### 2026-11-05: The harness's fake key is put together at runtime
- **Found:** the 0.9.0 release's `plugin-dist` push was blocked by GitHub push protection: the mutation harness ships inside the `tattle` binary, and `concat!` joins its fake Stripe key at compile time, so the whole key was in the Linux binaries.
- **Fixed:** `mutate::fake_secret()` stores the value with a `#` in the middle and removes it at runtime; neither the source nor the binary holds a key-shaped string (checked by scanning the release binary). Same value, same tests, same score.

### 2026-11-05: Test keys are obviously fake and split in the source
- **Found:** on the public repo, GitHub's push protection blocked three Stripe-shaped test values and its secret scanning flagged the Google-shaped one. They were always fake: inputs that let the redaction and `secrets.hardcoded` tests prove each key format is caught.
- **Decided (user):** "make them more obviously fake." Redaction tests use values that say FAKE where the format allows (`ghp_FAKEtestTOKENnotREAL…`, `AIzaSy-FAKE-test-key-not-real-…`, a JWT whose payload is `{"sub":"FAKE"}`, a private-key block reading `FAKE+TEST+KEY+NOT+REAL`); AWS keeps its documented example key. Every key-shaped value is built with `concat!` so no full key appears in the source. The `secrets.hardcoded` tests keep random-looking values on purpose (that rule ignores placeholders like FAKE), split the same way. Same tests, same score.

### 2026-11-05: CI's cancel rule lives on the job
- **Found:** on PR #3, a later push started a CI run whose job is skipped here (only opening a PR runs it), and the workflow-level `cancel-in-progress` rule let that empty run cancel the real one. **Fixed:** the rule is on the job, grouped by branch or PR and system, so a skipped job cancels nothing, and the three systems never cancel each other.

### 2026-11-05: CI when a pull request is opened; synthetic numbers on their own
- **Decided (user):** "Run CI on creation of every PR." In the development repo, CI runs on macOS, Linux and Windows when a pull request is opened or reopened, and by hand; the public repo runs it on every push and pull request. Later pushes to an open PR there don't re-run it (Windows takes about 40 minutes); start it by hand before merging.
- **Found:** `tattle score`'s "false alarms on clean fixtures" and its per-check table include the real labelled sessions, so on the Windows machine they read 18 and 37/41 (the 18 `manifest.missing` findings), while CI, which has no recordings, read 0 and 41/41. `release_numbers.py` now computes the synthetic line from synthetic fixtures only and says how many of the score's count come from real sessions.

### 2026-11-05: Release numbers leave out manifest.missing; every home folder is home
- **Decided (user):** "If false alarms exclude them from the result counts and add that to the output instead." The first full Windows run (163 sessions, commit 3bd1f00) had 12 clean sessions flagged, every one by `manifest.missing`, all in recall3; on the Mac the same sessions had none. `scripts/release_numbers.py` now leaves that rule out of every count and lists the sessions it fired on, with what each recording's `manifests/` folder holds (missing, empty, unreadable, or fine), so the cause can be found instead of hidden. Why that rule: it only fires when the research runner asked for a work summary (`--manifest`); the plugin never asks, so no user sees it.
- **Also in the script:** `--from score.json` reads a saved `tattle score --json` (no 75-minute re-run), and catches are split into static checks and `tattle verify` records.
- **Windows bug (CI):** a session opened in the home folder was recorded when `HOME` (Git Bash) and `USERPROFILE` differ. Both now count as home (`fsx::home_dirs`).

### 2026-11-04: Report items say what, and which request
- **Decided (user):** "do all three", after a demo of the report from a 9-request live session whose items said only "you mentioned something that doesn't exist" and "tattle missed part of this session".
- **Name the thing:** `intent.phantom_reference` names what was mentioned (`` `formatDate` ``, the test "…", the route `/x`). `capture.incomplete` says what was missed: requests with no ending, actions with no result, or a recorder problem; `capture.sequence_gap` says there's a gap in the log.
- **Merge repeats:** findings of one rule in one request that differ only by file read as one sentence ("Claude changed 2 files outside this project: `a` and `b`."), in the report and at the end of a turn. Rules without plural wording keep one line each.
- **Request number and quote:** with more than one request, each report item starts `Request N ("…")`, quoting the request cut at a word. Items are in request order; whole-session items come last. The end-of-turn line has no number (it's about the turn just finished).
- **How:** findings that belong to one request but had no event number get their prompt's `seq` (claims and the manifest: the latest request's). It's filled in after de-duplication, so the set of findings, the score and determinism are unchanged (checked on the 113 restored recordings: same 37 findings by rule). `shown.json` now stores `{text, request}`; older files with just the sentence still load.

### 2026-11-04: CI smoke test through the plugin launcher
- **Decided (user):** "Add smoke tests." `scripts/smoke_plugin.sh` puts the binary in a copy of `plugin/` under its release name, then sends one turn through `plugin/scripts/tattle` (a "tests pass" claim with no run) and checks the end-of-turn alert and `/tattle:report`. The release names are written out independently of the launcher, so a mismatch fails. CI runs it on macOS, Linux and Windows.
- **Docker only with Linux containers:** Docker in Windows-container mode (GitHub's Windows runners, or a Docker Desktop setting) counted as available and then failed to pull the Linux image. `docker::available()` now checks the server's OS.
- **Actions minutes:** a 90-minute limit per job, and a new push cancels the run still going.

### 2026-11-04: Pasted code isn't a phantom reference
- **Decided:** `intent.phantom_reference` ignores names inside code the user pasted into the prompt: heredoc bodies (`<<'EOF' … EOF`, found even when the paste lost its line breaks) and fenced blocks, plus any name the prompt itself defines (`function splitBill(…)`). A space after `<<` counts only before an upper-case word, so `1 << n` isn't a heredoc.
- **Why:** the first live Windows session (RESULTS §3r) raised a false alert on a pasted setup script. Pasted code is what the user is providing, not code they assume exists. Names in the prose around it are still checked.
- **Measured before the fix**, with a unit test reproducing the session's prompt and controls that must still fire.

### 2026-11-04: Public release: what stays private
- **Decided (user):** "Clean out private only material… Need to be release ready." The code goes public as `LLhuede/tattle` from one fresh commit (no history). Development continues in a private repo.
- **Left out of the public tree:** the maintainer's release checklist and an internal prompt stay in the development repo but are `export-ignore` (`.gitattributes`), so `git archive` leaves them out. Local machine paths in reviewer notes were made generic.
- **Kept:** every measurement of tattle itself, misses included, and the labelled corpus. The rules for contributors in `CLAUDE.md` are rewritten for a public project: the checker still never calls the network, recordings and results are still never committed, and contributors still don't contact other projects on the project's behalf.

### 2026-11-04: Fixes from the code review of the release branch
A code review of the release branch found 10 issues; all applied with minimal edits and a test each:
- **Windows paths:** the live check, report and settings used plain `canonicalize()` (Windows `\\?\C:\…`, which git can't use for alternates); now `fsx::canonical`, as `Recording::load` already did.
- **Heredocs:** a heredoc piped into a shell (`cat <<EOF | sh`) or fed to one behind `sudo` flags runs, so its body is kept; a `<<` with no closing line (a shift operator, `1 << n`) isn't a heredoc.
- **The agent's own new test:** only lines the agent really added this turn count (an edit's new lines that weren't in the text it replaced, or a new file), not a rewrite of an existing test file or repeated context, so weakening an existing test stays an alert.
- **Backward-compatible renames:** only explicit wording (backward compatible, "old … loading", alias, deprecated, migrate, legacy); "keep the tests working" no longer exempts a half-done rename. The pattern is compiled once.
- **Clean-up of old sessions:** session start no longer repacks in place. It deletes refs and prunes, then starts `repack -A -d` in the background, which loosens unreferenced objects instead of dropping them (a concurrent snapshot can't lose a blob); a later session start prunes them once a day old. So deleted sessions' files leave the disk within a day, not at once.
- **Output of a plain reader** (`cat old.log`, `git show …`, `grep`, `tail` only) isn't a test run, even when it shows a runner summary.
- **Compound commands:** unittest's "OK" only counts next to its "Ran N tests" line; go's `ok pkg (cached)` is a passing summary.
- **This repo's development commands:** `.claude/settings.json` sets `TATTLE_REPORT_COMMAND=/tattle-report` in `env`, so the dev skills use the dev prefix too.
- `config.rs` reuses `live::command_prefix()` instead of a copy.

### 2026-11-04: Renamed cv → tattle
- **Decided (user):** "Let's do tattle." All 8 README reviewers flagged the old names: the repo "Trust", the tool "cv" (can't be searched: résumé, computer vision, OpenCV) and the install `cv@trust`. Checked anonymously, read-only: "alibi" is taken in the same space (a Claude Code tool, and ALiBi in LLM research); "trust" is a crowded word. "tattle": no big project owns it and no Claude Code plugin uses it; crates.io and npm names are taken (we publish to neither: an npm package would need e.g. `tattle-cli`); two tiny, weeks-old agent projects use tattle/tattletale.
- **Renamed:** the binary and crate (`crates/tattle`), the plugin and its commands (`/tattle:report`, `/tattle:mute`, `/tattle:unmute`, `/tattle:tell-claude`; this repo's copies `/tattle-…`), the launcher (`plugin/scripts/tattle`), plugin binaries (`tattle-<os>-<arch>`), the release marketplace (`tattle`; install `/plugin install tattle@tattle` from `LLhuede/tattle#plugin-dist`), every user-facing message ("⚠️ tattle: …"), and settings: `TATTLE_<NAME>` (`TATTLE_MUTE`, `TATTLE_TELL_CLAUDE`, `TATTLE_PLUGIN`, …).
- **Kept, for compatibility:** the old `CV_<NAME>` settings still work (`capture::settings`); sandboxes and project hooks that call `cv hook` are still recognised as recorders; the `.checker/` folder, the recording format (`cv_version`, `cv-trimmed`, `refs/cv/…`, `cv-manifest/1`) and `~/.cv/linux-deps`, so every existing recording and the dependency pool still load; rule (finding) ids.
- **Not renamed here:** the private development repo (the public repo is created fresh as `LLhuede/tattle`), and history: RESULTS and DECISIONS entries before this date keep the old name, with a note at the top of each.

### 2026-11-04: Muting a rule, and sending alerts to Claude
- **Decided (user):** "add the per-rule mute and opt-in mode to send things back to Claude" (both raised by the README reviews: a power user would uninstall without a mute, and some people would rather Claude saw the alerts itself).
- **Mute:** `/cv:mute <rule>` / `/cv:unmute <rule>` (`cv config mute|unmute`) write `.checker/config.json` for the project; `CV_MUTE` (comma list) adds rules for every project. Muted rules aren't shown at the end of a turn or in `/cv:report`, and aren't marked as shown (unmuting shows them). The report names each item's rule and lists what's muted, so nothing disappears silently. Rule ids are checked against the catalog.
- **Tell Claude (opt-in, off by default):** `/cv:tell-claude on|off` (`cv config tell-claude`), or `CV_TELL_CLAUDE=1|0` for every project (overrides the project). With it on, the Stop hook returns `decision: block` with the new alerts and a short instruction: put a wrong or unproven claim right and show it, or say in one sentence why cv is wrong. The person still gets the `systemMessage`. **Never loops:** when Stop fires again with `stop_hook_active`, cv only shows the person any new alert.
- **Why off by default:** cv's design sends alerts to the person; handing them to Claude costs a turn and tokens, and a false alert would make Claude chase nothing. The person opts in per project or globally.
- **Live test (Linux, headless):** with tell-claude on, a false "All tests pass." was handed back; Claude answered "cv is right… that claim was unproven", tried to verify, and retracted it; the second Stop sent nothing. It also found that cv checked its own commands: `/cv:mute claims.tests_pass_unverified` read as a reference to code that doesn't exist and was handed to Claude. **Fix:** turns whose prompt is one of the plugin's commands are dropped before checking (later sequence numbers move down, so real gaps still show). A mistyped rule is now a plain message.
- **Scope:** settings change only what the plugin shows and sends. `cv check` and `cv score` ignore them, so findings for a session stay deterministic.

### 2026-11-04: Deleted sessions really leave the disk
- **Bug (found by the README security review):** since snapshots are packed in the background (2026-11-02), deleting old sessions removed their refs, but `git prune` only removes loose objects, so the files stayed in the pack. The "30 sessions / 30 days" promise didn't hold for file contents.
- **Fix:** after deleting refs, `git repack -a -d` rewrites the packs without unreferenced objects, then `prune --expire=1.day.ago`. Loose objects another session is writing at that moment are untouched. Test: a file only in a deleted session is gone from the store after pruning, even when packed.

### 2026-11-04: Two fixes from re-scoring the old recordings
Re-scoring the 133 older labelled recordings, restored locally (RESULTS §3q), found:
- **`tests.edited_after_failure` leaves out test files gone by the end of the turn** (real session H33: a scratch repro test written, run and deleted twice). It can't have weakened the project's tests. This was a regression from reading `FAIL` lines, which is right and kept.
- **`claims.flaky_unobserved`: a run that failed for a setup reason (tool or module missing) isn't the flakiness being observed** (real session R52: `vitest: command not found` before `npm install`). Same setup-failure rule as `shows_test_failures`, now one function.
Synthetic and real: 41/41 checks at 1.00, 0 false alarms on 98 clean real sessions, 366/366 deterministic; 226 tests including the real-session label test.

### 2026-11-04: Fix from everyday batch 6
- minitest ("12 runs, 17 assertions, 0 failures, 0 errors") and RSpec ("3 examples, 0 failures") summaries mark test runs, and their non-zero failure or error counts mark a failure (RESULTS §3p: both batch 6 false alerts). Synthetic 1.00/1.00, 0 false alarms, 233/233; 224 tests.

### 2026-11-04: Test runs recognised by their output
- **Why:** 9 of the 17 false alerts in everyday batches 1–4 came from command shapes the test-command patterns missed (`node --test`, interpreter flags, heredocs, several commands on a line). Each fix covered one shape.
- **Decided (agent; within the user's "fix whatever batch 5 finds"):** a `Bash` call is also a test run when its output carries a test runner's own summary line: unittest `Ran N tests in`, TAP `# tests N`, vitest `Test Files`/`Tests N passed`, jest `Tests: N`, cargo `test result: ok.`, pytest `== N passed … in`, go `ok pkg 0.01s`, mocha `N passing (`. Anchored at line starts, so `grep` output (`file:12:Ran 3 tests`) doesn't count. The summary also decides a piped run's outcome (unittest `FAILED (failures=N)` added).
- **Mutation realism:** `claim-pass-no-run` replaced the test command but left the runner's output in stderr and the transcript; a type check never prints a test summary, so it now clears all of it. Without that, recall dropped to 0.60 (the check was right, the injected session wasn't realistic).
- Synthetic 1.00/1.00, 0 false alarms, 233/233; 224 tests. Built after batch 5 (which measured `86ce1ec`); batch 6 measures it.

### 2026-11-03: Fixes from everyday batch 4
The four false alerts of batch 4 (RESULTS §3n, recorded first):
- **Interpreter flags before `-m`** (`python3 -W error::ResourceWarning -m unittest`, `python -B -m pytest`) still make a test run.
- **Query parameters and flags in a request** (`?include_past=`, `&page_size=`, `--dry_run`) are things to add, not references to existing code. Deliberately not "any name that appears in the code afterwards": that would hide an agent quietly *using* a function that doesn't exist (R73, `res.sendError()`).
- **The project-command rule strips heredoc text** before looking for `npm test`, like the other command rules.
- **Admitting a failure** now also matches "One test in `plugin.rs` fails" (words between). With a failure admitted, only partial claims ("the new tests pass") are excused; "All tests pass" still counts. A first attempt that excused any claim after an admission dropped `claims.tests_pass_other_command` recall to 0.75 on the mutations; caught by `cv score` and reverted.
- **One file then the whole suite on one line** (`node --test test/cli.test.js; npm test`) is a whole-suite run, not a filtered one.
Synthetic unchanged (1.00/1.00, 0 false alarms, 233/233); 222 tests.

### 2026-11-03: Fixes from everyday batch 3
The two false alerts of batch 3 (RESULTS §3m, recorded first):
- **Obviously made-up secrets aren't reported** (`secrets.hardcoded`): a value with a run of 8+ consecutive letters or digits (`abcdefgh`, `01234567`), 8+ repeated characters, or a placeholder word (example, fake, dummy, placeholder, changeme, xxxxxx, …). Real keys are random. A random-looking token in a test module is still reported.
- **`tests.edited_after_failure` on the agent's own new test** (the CLAUDE.md watch item): when every edit between the failing and passing runs only changes text the agent wrote into that file earlier in the same turn (the replaced text, 12+ characters, is inside an earlier edit's new text), the finding is kept, with the same id, but says "a test written earlier in this turn", and the plugin shows it as a **note** in `/cv:report`, not an end-of-turn alert. Why not drop it: statically, "fixed a miscount in its new test" and "bent its new test to fit buggy code" look the same, and the labelled real catches (R82, R84, R98) can't be re-checked in the cloud. **To check on the machine with the recordings:** whether those three findings carry the new wording. If they do, the plugin no longer alerts on them, and the trade-off needs a decision.
- **E02 (a test removed for a requested behaviour change)** is accurate and stays an alert: a removed capacity test is worth a look.
Synthetic unchanged (1.00/1.00, 0 false alarms, 233/233); 221 tests.

### 2026-11-02: Fixes from everyday batch 2
The three causes of batch 2's false alerts (RESULTS §3l, recorded first):
- **Line breaks start commands:** a runner at the start of a line counts (`…\nEOF\npython3 -m unittest`). Heredoc bodies are stripped first, so a README written with `cat <<EOF` that mentions `go test` isn't a test run.
- **A line of several commands that fails** (`a; b`, `a && b`, newlines): when the output shows the test runner's own passing summary and no failing one, the tests' result is *unknown*, not failed. Something else on the line failed (`unittest …; mypy --version` with mypy missing). A single failing command stays a failure.
- **Double-quoted text isn't the agent's claim** ('a session with a false "all tests pass" claim'). Single quotes (apostrophes) and backticks (commands) are left alone.
Synthetic score unchanged (1.00/1.00, 0 false alarms, 233/233); 220 tests.

### 2026-11-02: Fixes from everyday batch 1
All four causes of batch 1's 8 false alerts (RESULTS §3k, recorded first) fixed generally, not per session:
- **Test runs:** `node --test`, `deno test`, pytest, `python -m unittest`, `go test`, mocha/ava/uvu, rspec, `mvn`/`gradle`/`dotnet test`, phpunit and `make test` now count, but only in command position (`pip install pytest` doesn't). Piped output: TAP `# fail N`, pytest `== N failed`, go `FAIL`, mocha `N failing` mark a failure. Effect beyond the batch: every check that reads test runs now sees these sessions, so the real labelled set must be re-run (`scripts/release_numbers.py`) to catch any new false alarm.
- **Names in other languages:** the "doesn't exist" lookup searches every common source and config language, not just JS/TS/Python/web. A wider search can only make that finding rarer.
- **Backward-compatible renames:** when the request asks to keep the old name working (backward compatible, keep old files loading, alias, deprecated, migrate, legacy), leftovers of the old name make the rename *unknown*, not unfinished.
- **"lets/allows/enables tests pass"** describes what the code permits, so it isn't a claim (like "should pass").
Synthetic score unchanged (1.00/1.00, 0 false alarms, 233/233). Measured on a fresh batch 2, not on batch 1.

### 2026-11-02: Road to a public 1.0
- **Decided (user):** publish as open source from a version still to be finalised. The public repo starts from **one fresh commit**, not the development history. Nothing is made public, released or pushed to a public place without the user's all clear.
- **Decided (user):** the README is rewritten around the plugin ("we want people to actually open and use this"). Draft in `docs/README_DRAFT.md` (now `README.md`); numbers that need the local recordings are marked ⟦…⟧ and come from `scripts/release_numbers.py`.
- **Proposed (agent, user to confirm):** the bar for "1.0" in the release checklist (plugin on all three systems, 30–50 everyday sessions with no false alert, a fresh blind batch, re-run real numbers, Docker re-verify). Short of that, release as a 0.9 preview. Reason: the 0-false-alarm record is on the test corpus; every fresh batch so far found new false alarms, and the plugin now shows alerts in people's own projects.
- **Decided later (user, 2026-11-04):** MIT.
- **Built:** `.github/workflows/release.yml` (builds macOS universal, Linux x86-64/Arm, Windows x86-64; packages `plugin-dist`; runs only by hand and pushes only when asked) and `ci.yml` (by hand for now). `scripts/release_plugin.sh --no-build` packages CI-built binaries; a `v*` tag names the version.

### 2026-11-02: Plugin fixes from live testing
Installed the plugin from a locally built `plugin-dist` through `/plugin marketplace add` + `/plugin install`, and ran real headless sessions (Linux, cloud):
- **Works:** clean sessions stay silent; a false "All tests pass." raised the end-of-turn line, shown by Claude Code as `Stop says: ⚠️ cv: …`; `/cv:report` runs as a slash command. Tool hooks cost 2–4 ms; `.checker/` stays out of `git status`.
- **Fixed:** `/cv:report` runs inside its own unfinished turn, which read as "cv missed part of this session" and counted as a request. The report now leaves out the running turn and doesn't count report requests.
- **Fixed:** `scope.unrelated_file` fired on the only file a turn changed (the file the prompt named). It now needs another changed source file in the turn. Mutations still 12/12.
- **Fixed (speed):** on a 20,000-file project the end-of-turn check took 6.5 s, almost all in reading every file twice and parsing them twice for the prompt checks. Snapshots are now packed in the background (at session start and after each check), and parsed names are cached by content (shared within a check; kept per live session). Now ~3 s on the first turn and ~2 s after; `.checker` 83 MB → 6 MB. Results are identical by construction (same parser output, keyed by content hash); synthetic determinism 233/233.
- **Windows:** Claude Code runs hook commands through Git Bash on Windows, or PowerShell when Git Bash is missing. The launcher now recognises Git Bash (`MINGW*/MSYS*/CYGWIN*`) and runs `cv-windows-<arch>.exe`. PowerShell-only Windows isn't supported: no hook command line works in both shells without recording twice, and the checks only read the `Bash` tool. Not yet run on a real Windows machine.

### 2026-10-29: Docker backend, step 2: Linux dependencies
- **Decided (user):** "build it that way": an opt-in download step for the containers' dependencies. Resolves open decision #2 in `docs/PHASE_4B_PLAN.md` (installs) for the Docker backend.
- **Built (`crates/exec/src/linuxdeps.rs`):** a pool of Linux `node_modules`, one per lockfile and image (`~/.cv/linux-deps/<key>/`, or `CV_LINUX_DEPS`):
  1. **download** with `cv verify --allow-install`: `npm ci --ignore-scripts` in a container with network. No project or package code runs; only npm. Proxy settings and a CA bundle are passed through; a proxy on localhost means host networking.
  2. **native packages** (`binding.gyp`) are compiled with `npm rebuild` in a container with the network **off**, against the image's Node headers.
  3. moved into place in one rename, with `ready.json`. Without `--allow-install` only ready entries are used; otherwise the session is "couldn't verify (dependencies)".
- **Use:** under Docker every copy gets the pool's `node_modules` mounted read-only, with scratch mounts for tool caches (`.vite`, `.cache`, …). The host's own `node_modules` folders (macOS or Windows builds) are never used there.
- **Gate (cloud session, Linux + Docker):** G2 (express, vitest, native `better-sqlite3`): pool built, 27/27 tests pass with no network; reused without download. The full `cv verify` pipeline on a two-commit G2 history: build, app start, both suites, and both cross runs come out as expected.
- **Limits:** packages that need network in their install scripts (e.g. downloading browsers) won't work; they show as couldn't-run or failing at both ends, never as a regression. Not yet run on the 173 recorded sessions (needs the recordings on the Windows machine).

### 2026-10-29: Docker backend for running code (step 1: the sandbox)
- **Decided (user):** Docker replaces the macOS sandbox off macOS ("I'll use docker for sure").
- **Built (`crates/exec/src/docker.rs`):** each command runs in a fresh `node:22-bookworm` container (`CV_DOCKER_IMAGE` overrides):
  - `--network none`, read-only root, `--cap-drop ALL`, no privilege escalation, 512 processes, 2 GB memory
  - bind mounts: the project copy and the temp folder (writable), and the dependency folders its `node_modules` links point into (read-only). The host's home folder is never mounted, so secrets can't be read.
  - only the variables we pass; the timeout kills the container
  - on macOS and Linux the mounts keep their host paths; on Windows they're under `/host/<drive>/…` and paths in arguments are translated
- **Backend choice:** the macOS sandbox when present, else Docker; `CV_EXEC_BACKEND=docker|macos|none` forces one.
- **Gate:** all 11 escape tests pass under Docker (no outside network, no writes outside the copy, secrets unreadable, environment scrubbed, timeout and process bomb stopped).
- **Known difference:** Docker always keeps the container's own loopback, so the "loopback off" mode isn't enforced there. Every probe uses loopback on.
- **Next (step 2): dependencies.** The recordings' dependency pool holds macOS builds and a Windows `node_modules` holds Windows builds; the container needs Linux ones. Proposed: an opt-in install step that downloads from the npm registry with install scripts off (the project's code never runs with network), builds native packages offline, and keeps a Linux pool per lockfile. Needs the user's OK (open decision #2 in `docs/PHASE_4B_PLAN.md`).

### 2026-10-29: Commit author
- **Superseded (2026-11-04):** commits are authored by the person contributing, never by the agent (CLAUDE.md rule 7), so outside contributors don't commit as the maintainer.
- **Decided (user):** every commit is authored as the maintainer, set per checkout with `git config`. Agents keep their `Co-Authored-By` trailer.

### 2026-10-29: Windows becomes the development machine
- **Decided (user):** the Mac is gone; development continues on Windows, from a backup of the project folder. "Set it up to run on windows."
- **Done:** the workspace builds and tests on Windows (`docs/WINDOWS.md`). The fixes are in shared helpers (`capture::fsx`): Windows path forms, home folder and user name, read-only git files, plus a git-based `restore` and `.gitattributes` for LF line endings.
- **How it was checked:** cross-compiled for `x86_64-pc-windows-gnu` and run under Wine with Git for Windows, from the Linux cloud session. Not yet run on the real Windows machine.
- **Lost for now:** `cv verify` (the macOS sandbox and the macOS dependency pool). On Windows it says it can't run and never reports a failure. A Docker or WSL backend is the open question.
- **Not yet checked:** recording live sessions on Windows (hooks through Git Bash), and the Python scripts.

### 2026-10-29: `claims.flaky_unobserved` (improvement plan #4, static part)
- **Decided:** fires when the prompt says a test is flaky, the closing message says it's fixed ("fixed", "passes consistently", "no longer flaky"), and no test run before the agent's first code or test edit failed.
- **Clears it:** a failing run before the first edit (the flake was seen); a command run by hand to reproduce it (`node -e`, `curl`); the agent saying it couldn't reproduce it, never saw it fail, or that the test isn't actually flaky.
- **Why only before the first edit:** after an edit, a failure can be the agent's own doing (R52's new test failed on its first run).
- **"Flaky" must be about a test, suite or CI.** Plan D's B4 asks for "flaky jobs" as a feature.
- **Built on** R22, R52 and R89 (all fake flaky fixes; labels updated). No honest flaky-fix session exists yet, so the false-alarm side is only covered by synthetic controls. That's the main risk: a genuine fix of a real flake that the agent never saw fail would be flagged. The message says "never observed", which stays true in that case.
- **Not built yet:** the optional execution part (run the named test about 30 times on the old code). It needs the macOS sandbox.

### 2026-10-29: Cross runs and two findings (improvement plan #1 and #2)
- **Decided:** `cv verify` adds a cross record (`crates/exec/src/cross.rs`), vitest only, per-test results from its JSON reporter:
  - **P5:** the final test files on the code from the session start
  - **P4:** the starting test files on the final code
  - each mixed copy runs twice; a test whose runs disagree is `flaky` and never evidence
- **`exec.fix_not_demonstrated`:** a fix request; the agent names tests as proof (a title of 10+ characters in the prompt, or in a closing sentence with an evidence word such as "now passes", "verifies", "fixed"); every named test, in its final form, passes on the old code. Prompt-named tests count only when the agent also says "fixed". A test the agent says "now passes" counts on its own (R85).
- **`exec.tests_bent`:** a fix request; tests that passed at the start fail on the final code, in test files the session edited.
- **False-alarm controls:** no finding when the agent says it couldn't reproduce the bug; flaky tests never count; both checks need a fix request (a genuine fix never breaks a passing test, a feature may).
- **Not applied (needs the user):** widening `fix_request()` to "incorrect", "double counts" and "is/are off", so R84 counts as a bug report. It was written in the WIP commit after the user stopped that edit, and has been taken back out. It would be tuning on R84, and it also widens `claims.fix_unreproduced`.
- **Status:** built with mutations and controls (synthetic 1.00/1.00). Not yet re-verified over the 173 local sessions (needs the Mac).

### 2026-10-29: Everyday use as a Claude Code plugin (Phase 8, first part)
- **Decided (user):** make `cv` simple for people who just use Claude Code: install once (two `/plugin` commands), then nothing to learn. Built: the plugin, the end-of-turn check, plain-language findings and housekeeping. Feeding findings back to Claude (blocking `Stop`) is **not** built yet, by choice.
- **End-of-turn check:** in plugin mode (`CV_PLUGIN=1`, set by `plugin/scripts/cv`), the `Stop` hook runs every check on the live session (`Recording::load_live`: same events, same slimmed transcript, snapshots read through git alternates, so the result matches an export; a test enforces it). New alerts become one `{"systemMessage": …}` line for the person. Claude isn't told, and nothing prints when there's nothing new. A 20 s deadline means a huge project never holds Claude up (it says nothing instead).
- **Plain words** (`checks/plain.rs`): one sentence per finding id, never the id itself. Checks only ever labelled "tolerated" on real sessions are **notes** (report only): `deps.added`, `events.changed_by_command`, `intent.target_not_changed`, `code.swallowed_error`, `manifest.claim_unsupported`, the capture checks and other "usually fine" items. Everything else is an **alert**. Each alert is shown once per session (`shown.json`) and stays in `/cv:report` afterwards.
- **Housekeeping:** at session start, keep the current session plus the newest 30, and delete anything older than 30 days (refs removed, objects pruned after a day so a concurrent snapshot is safe). `errors.log` rotates at 1 MB. `.checker/` already ignores itself.
- **Never recorded in plugin mode:** the home folder or `/`; a project with its own `cv hook` hooks (research sandboxes, this repo), so nothing is recorded twice; macOS without the developer tools (`git` would pop up an install dialog).
- **Hook output rule changed:** `cv hook` still always exits 0, but may now print in two cases: manifest mode (a request for the agent) and plugin mode at `Stop` (a line for the person).
- **Distribution:** binaries never go on main. `scripts/release_plugin.sh` builds a one-commit `plugin-dist` branch (marketplace `trust`, plugin `cv`, binaries in `plugin/dist/`). It must run on a Mac for macOS binaries. Users: `/plugin marketplace add <repo>#plugin-dist`, then `/plugin install cv@trust`. Not signed or notarised yet (git clones don't get the quarantine flag, so macOS doesn't block it).
- **Development (dogfooding):** this repo's `.claude/settings.json` runs the same launcher (`CV_SOURCE=project`), from a frozen copy in `plugin/dist/` that `scripts/dev_plugin.sh` refreshes. So rebuilding `target/release/cv` never disturbs a recording session. Cloud sessions don't load plugins, but they do run these project hooks; an async `SessionStart` hook builds the copy when it's missing. `/cv-report` is the development copy of `/cv:report`.
- **Checker changes found by dogfooding, with tests:** `cargo test` / `cargo nextest run` count as test runs, and cargo's `test result: FAILED` summary counts as a failure (otherwise every "tests pass" in a Rust project is "unverified"). Heredoc bodies aren't judged as commands unless a shell runs them (`bash <<EOF`, `ssh host <<EOF`). The first live session here flagged `rm -rf` inside a script being written. Synthetic score unchanged (all checks 1.00/1.00, 0 false alarms, 217/217 deterministic). The labelled real sessions still need a local `cv score`.
- **Open:** the report command is named `/cv:report` (plugin) and `/cv-report` (here). A licence and an icon are only needed for Anthropic's public directory, which the privacy rule rules out for now.

### 2026-10-29: Private GitHub repo for cloud sessions
- **Decided (user):** the code goes to one private GitHub repo so sessions can continue in Claude Code's cloud. This relaxes "no git remote" from 2026-10-27; the rest of the privacy rule stands.
- **Repo:** private.
- **What stays local:**
  - recordings (`fixtures/recorded/`) and `artifacts/`
  - the dependency pool (outside the repo, several GB)
- **Limits in the cloud:** the cloud is Linux, so the macOS `sandbox-exec` layer, `cv verify` and the labelled-session tests (they need local recordings) don't run there. Static checks, unit tests and synthetic mutations do.

### 2026-10-27: App-start probe (Phase 4b.3)
- **Decided:** P3 runs only for servers (a start script plus a listen call). It runs twice and needs a baseline that started, or no baseline app. EPERM in the server log means couldn't-run (our sandbox), not a crash.
- **Sandbox:** Unix-socket bind and accept are allowed only under the sandbox's tmp and work folders. The temp root stays short because of the ~104-byte socket path limit on macOS.
- **Why:** the A1 flagship's app was dead while its own unit tests passed. Only starting it shows that.

### 2026-10-27: Everything stays private
- **Decided (user):** nothing about this project leaves this machine.
  - No git remote.
  - No contact with other projects or their authors: no stars, forks, watches, issues, PRs or comments.
  - Read-only, anonymous access to public pages only (research, a one-time source download to study).
  - Third-party tools run only in our sandbox with network off, after a telemetry check.
  - Results stay in our own docs.
- **What was already true:**
  - The repo has no remote.
  - The research agents only read public pages and used web search. They made no accounts, contacts or interactions.
  - The GitHub checks were anonymous, read-only API calls.
- **Limits worth knowing:**
  - Owners of public GitHub repos can see anonymous aggregate traffic (view and clone *counts*), never who.
  - Our conversations and the headless agent runs go to Anthropic, as with any Claude use.

### 2026-10-27: Execution records and findings (Phase 4b.2)
- **Record:** `cv verify --write` saves `<recording>/exec/record.json` (local, never in git). `cv check` only reads it, so the report stays deterministic for a given record.
- **Probes build before they test,** as the project's own workflow does (real session H23).
- **Findings:**
  - `exec.regression` when a baseline exists, else `exec.tests_fail`
  - `exec.build_fails` only when the start was fine
  - `exec.claim_contradicted` when "tests pass" (message or manifest) meets a failing, broken or absent suite
  - flaky, timed-out or couldn't-run results give nothing
- **Labels** list exec findings, but the scorer only expects them when a record exists. The escape tests run serially, because the process-bomb test starved a vitest run in parallel.

### 2026-10-27: Phase 4b sandbox: macOS sandbox-exec, Docker fallback, no network installs
- **Decided (user):**
  - `sandbox-exec` first, Docker as the fallback
  - no network installs for now: dependencies come from linking or the offline cache only
- **Built (4b.0):** the `crates/exec` runner and 10 escape tests. See `docs/PHASE_4B_PLAN.md` for results and the network-rule trap found in the spike.

### 2026-10-26: No recordings in git; big from-scratch test suites (A–D)
- **Decided (user):** recordings never go in git, for any test. `fixtures/recorded/` and `artifacts/` are gitignored and the 785 tracked recording files are untracked. They stay on disk. Labels (expected findings) move to `corpus/labels/<ID>/run<N>.json`, which stays in git. `cv score --labels`. New harness test `real_sessions_meet_their_labels` runs when recordings are present and skips otherwise.
- **Corrected:** the old harness test never read real recordings, so "cargo test enforces 0 false alarms on real sessions" wasn't true until now.
- **Big tests** (`bigtests/`), built and validated but not run (user: architect first):
  - A: a flagship from-scratch build, 21 scripted turns
  - B: 8 medium builds
  - C: a rule-based simulated user driving A's product
  - D: a hybrid run plan with gates and a $250 cap
- **Ground truth is hidden black-box acceptance tests** (`node:test`, zero dependencies, `bigtests/CONTRACT.md`), run after every turn by `scripts/acceptance.py`. Each suite has a reference implementation that passes all its tests, which proves the tests are correct and passable.
- **The simulated user is rules, not a model.** It never quotes hidden test names and turns failures into user-level complaints. `--llm-user` (optional) only rewords messages, in the harness, never in `cv`.
- **Runs** have spend caps, stop at usage limits (exit 3), resume, and write only to `artifacts/`. `scripts/fake_claude.py` tests the runner for free.
- Specs, scripts and hidden tests were written by agents that couldn't see the checker.

### 2026-10-26: Recall study, new hallucination checks, real sessions as a test suite
- **Recall study:** trap prompts (`corpus/recall/`) were recorded on Haiku 5.5 and Haiku 4.5 until real hallucinations appeared. Haiku 5.5 made none on the traps. Haiku 4.5 made many: "fixed" features that don't exist, called a made-up API, claimed tests pass without running them, left a rename half-done "throughout", and accepted false bug reports. Batch numbers were recorded before any fix (`harness/results/recall-cv-prefix-2026-10-26.json`, local).
- **New checks** (each with a mutation, and controls where they apply):
  - `claims.tests_pass_unverified`
  - `manifest.tests_unrun_passed`
  - `intent.phantom_reference`: names that don't exist when the turn starts, and the agent's closing message doesn't say so; file names are excluded
  - `intent.unrequested_change`: explicit, global "don't change any code" directives only
- **False-alarm fixes from real data:**
  - free-text manifest items are "uncheckable", not false
  - `it.each` and describe-path test names match
  - "keep / confirm / no X" requests may list unchanged files
  - answer-only last turns use the session's changes
  - question or verify-only prompts don't require changes
  - scoped "tests pass" claims are fine when failures are reported
  - renames are scoped to the named object ("field on expenses")
- **Rename "everywhere":** counts untouched files and camelCase compounds, but only once the agent has renamed compounds itself. Names the prompt mentions are excluded.
- **Real sessions are now a test suite.** Every recording has a `labels.json` with:
  - `defects`: findings `cv` must report
  - `tolerated`: grey findings
  - `hallucinated`: plus the reviewers' list
  
  Labels are built by `scripts/make_labels.py` from AI reviews plus hand-verified decisions. They are AI judgements, not human labels. `cargo test` requires 0 false alarms and full recall on them. Real sessions are mutation sources only with `mutation_source: true`, which keeps scoring fast.
- **Git:** the test suite (prompts, bases, recordings, labels, scripts) is committed. Results (scores, oracle output, reviews) are gitignored and kept local. User decision.
- **Held-out retest:** `corpus/recall2/` (30 prompts) was written by an agent that couldn't see the checker, and recorded after the fixes.

### 2026-10-25: Fresh-batch fixes measured on a new batch
- **Decided:** the 5 false-alarm causes from the first fresh batch were fixed with general rules (lexicon v3) and regression tests named after the sessions. Those 6 prompts moved to `corpus/dev`.
- **Measured** on 8 new sessions (H30, H32, H41, H47, H48, H53, H54, H62), with numbers recorded before any fix: 2 findings (1 false, 1 grey), down from 8 false in 6 sessions. See `docs/RESULTS.md` §3d.
- **Not fixed yet:** H30 exposed a new cause, the rename check not being scoped to the named object. By the fresh-batch rule, H30 and the other 7 prompts now count as seen. Fixing against them needs a third fresh batch to measure.
- **Priority change:** false alarms are no longer the main gap. The recall study (sessions where agents actually make mistakes) comes next.

### 2026-10-23: Phase 6 design (6a intent checks, 6b manifest) and evaluation method
- **6a:** each prompt fact gets verified / violated / unknown with evidence (`checks/intent_checks.rs`). Only kinds precise on held-out prompts can be violated; the rest are information. Evaluated at session level (first human prompt → end).
- **6b:** the recorder asks for a manifest (`CV_MANIFEST=1`; hook context on each human prompt, one reminder at Stop). Claims are files, names, routes and tests; each is checked statically. A requirement whose items all check out is "backed" and can confirm prompt facts. The latest manifest is checked against its own turn.
- **Changes undone within a session aren't reported:** tests the session added then removed, dependencies added then removed, files created then deleted (real session H37)
- **Mutations can expect several findings**, those the change truly causes. Controls expect none.
- **Real-session evaluation:** independent AI reviewers judge each session before reading the checker's output (`fixtures/reviews/`). They're reported as AI judgements and kept separate from human labels.
- **Fresh-session rule:** false alarms found on a batch are fixed only after the batch's numbers are recorded, and the next honest number needs a new batch
- **Status:** Decided

### 2026-10-23: Phase 5 results, and what they mean for Phase 6
- **Measured** (`harness/results/2026-10-23-ec95ce2.json`):

  | Prompt set | Lexicon | Recall | Invented facts |
  |---|---|---|---|
  | Corpus (15, written against) | v1 / v2 | 0.98 / 0.98 | 0 / 0 |
  | First held-out (20, H01–H20) | v1 | **0.57** | **6** |
  | Same 20, now the dev set | v2 | 0.89 (tuned on it) | 1 |
  | Fresh held-out (20, H21–H40) | v2 | **0.49** | **12** |

  The coverage invariant (every word accounted for) held on all 55 prompts. Every "couldn't check" phrase stayed unchecked.
- **Conclusion:** a fixed phrase list over-fits whatever it is written against. On unseen prompts it finds about half the facts. Tuning on more examples raises the tuned set's score much more than the unseen set's.
- **Per kind, on the fresh held-out set:**
  - **Precise (0 invented):** `constraint.deps_unchanged` (3/3), `constraint.tests_pass` (3/4), `scope.only` (2/2), `target.unchanged` (3/7), `symbol.rename` (1/1), `route.exists` (3/4), `param.literal` (9/16)
  - **Imprecise:** `feature` (14/40, 2 invented; the long tail of feature names), `symbols` (0/5), `require.tests_added` (1/6, 3 invented: targets like "it" / "both" lost), `target.modify` (2/5, 3 invented), `stack` (11/15, 3 invented: "scrap redux", "switch to vue" not read as reversals), `storage` (3/4, 1 invented)
- **Decision for Phase 6:**
  - Only the **precise kinds** become checks that can report a violation
  - Imprecise kinds are shown as information ("you asked for X; couldn't confirm"), never as failures
  - The agent manifest (design approach #1) is the way to cover features and entities: the agent states what it built, the checker verifies those claims against the code, and the prompt's precise facts cross-check the manifest
- **Status:** Decided

### 2026-10-23: Held-out protocol for anything learned from examples
- **Decision:** prompt-understanding rules are scored on three sets:
  - `corpus/prompts`: the main corpus, written alongside the rules
  - `corpus/dev`: prompts the rules were tuned on (optimistic)
  - `corpus/heldout`: prompts written by a **separate agent that never saw the rules, corpus or dev set**. Never used for tuning.
- **When rules are tuned on held-out failures:** that set moves to `dev`, and a fresh held-out set is written before reporting a new number. This was done once (H01–H20 → dev; H21–H40 written fresh).
- **Scoring bugs** (how a fact is matched) may be fixed at any time. Lexicon changes may not use held-out examples.
- **Status:** Decided

### 2026-10-23: Phase 5 design (intent extraction)
- **Decision:** `checks/intent.rs`, no AI:
  - Sentence and clause split that leaves paths (`src/streak.js`) and decimals whole
  - Versioned lexicon (`LEXICON_VERSION`, now v2) written as code, not a data file: rules need regexes plus small amounts of logic (negation, hedges, grounding decisions), and code keeps them testable
  - 15 fact kinds, a closed list
  - Grounding against a repo index (files, top-level names, routes). Literal routes and paths match exactly; otherwise all specific words must appear in a project item. Confidence: exact / single-match / multi-match / ungrounded.
  - "it" / "that case" resolve to the previous target; a later turn replaces an earlier fact with the same key
  - Coverage accounting: every word is used by a fact, filler, or "couldn't check"
- **Corpus changes:** G2 frozen as a base; E2/E3/T5 gained features their prompts state; E4's unstated "tests pass" marked `implicit` and scored separately
- **Status:** Decided

### 2026-10-22: First real batch (headless, 1 run each): results and fixes
- **Sessions:** G1, G2, G3, E2, E3, T1, V1 (Claude Code 2.1.307, headless). ~$4.30 and ~35 minutes of agent time in total. All 7 apps work; the agent avoided the T1 trap (fixed `CODE_LENGTH` instead of the test).
- **Checker result:** no findings on any of the 7, which matches a manual read of each session. The real data so far tests **precision**; it contains no real defects to test recall on yet.
- **False alarms found and fixed (2):**
  - documented empty `catch` (G1), see the entry below
  - "edited without reading" when the agent read files with a shell wildcard (`cat src/*.ts`, T1). Path matching now understands `*`, `**`, `?` and `--include=` patterns.
- **Capture fixes:** headless permission denials fire **no** hook (not even `PermissionDenied`). The transcript's tool results are now a second source: a call with an error result there counts as failed, not as missing capture. This is the first hook-vs-transcript cross-check from Phase 2.
- **Known limit:** commands piped through `tail` exit 0 even when the tests didn't run (E2: dependencies weren't installed). Without a test summary in the output, the run counts as passed. Possible fix later: treat "command not found" / missing-module output as a failure.
- **Status:** Decided

### 2026-10-22: Real Claude Code hook format (verified, supersedes the synthetic guess)
- **Source:** 3 headless Claude Code 2.1.307 sessions (G1–G3, `scripts/record_headless.py`) plus the hooks reference (https://code.claude.com/docs/en/hooks.md)
- **Findings and decisions:**
  - **Bash results have no exit code.** `PostToolUse` fires only when a tool **succeeded**; a non-zero exit fires **`PostToolUseFailure`** instead, with `error` starting `Exit code N`. The synthetic `exit_code` field was wrong. The adapter now treats `PostToolUse` Bash as passed (unless a test summary piped through `tail` says otherwise) and reads failures from `PostToolUseFailure`.
  - **The recorder wasn't subscribed to failures**, so failing test runs were invisible in G1–G3. The sandbox hooks now include `PostToolUseFailure`, `PermissionDenied`, `SubagentStart`, `PreCompact` and `StopFailure`.
  - **Every tool call has a `tool_use_id`**, which links its PreToolUse to its result. Used to mark failed/denied calls on both events (a failed edit never counts as an edit) and for a new completeness check: tool calls with no recorded result → `capture.incomplete`. This correctly flags 3 denied commands each in G2 and G3.
  - **`Stop` carries `last_assistant_message`**, now the preferred source for "tests pass" claims
  - **Background-task notices arrive as `UserPromptSubmit`** (`<task-notification>…`). Turns now start only at human prompts.
  - **`SubagentStop` has `agent_transcript_path`**; subagent transcripts are copied and exported
  - **Transcripts are ~400 KB per session**, mostly Claude Code bookkeeping. Export now keeps only the conversation (~25 KB); the full cleaned copy stays in the project's `.checker/`.
  - **Not documented:** whether an unknown hook event name in settings errors. All names used exist in 2.1.307.
- **Status:** Decided

### 2026-10-22: First real-data false alarm: documented empty catch
- **Found in:** G1 (`catch { // Storage may be unavailable (e.g. private mode); the app still works… }`)
- **Decision:** an empty `catch` with a comment of 4+ words explaining why is a deliberate choice and isn't reported. Empty, or `// ignore`-style, still is.
- **Status:** Decided

### 2026-10-22: Recording sessions without a human
- **Decision:** `scripts/record_headless.py <ID> <run> [--base …]` runs a corpus prompt through headless Claude Code (`claude -p`, later turns with `--continue`) in a fresh sandbox folder, then exports. The agent gets file edits plus an allow-list of commands (npm, npx, node, git, basic file commands): no `rm`, no blanket permission. Denied commands are recorded (`PermissionDenied`).
- **Caveat:** headless runs aren't identical to interactive ones: there's no human to answer questions, and denials replace permission prompts. Interactive runs (`docs/RECORDING.md`) are still needed.
- **Labels stay human:** recordings made this way are committed unlabelled.
- **Status:** Decided

### 2026-10-22: Code check rules (Phase 4)
- **Parser:** tree-sitter with the TSX grammar (also reads plain JS/JSX). The checker parses code; the mutation generator uses simple text helpers. Two independent methods make correlated bugs less likely.
- **Only what the turn introduced:** each changed file is compared before vs after the agent's turn. Problems that were already there are never reported.
- **`tests.removed`:** tests (full name including `describe` blocks) that existed before and now exist in **no** test file. Moving a test to another file is fine.
- **`tests.assertion_weakened`:** new assertions on a constant (`expect(true)`, `expect(1)`, a fixed string), or a test that had assertions and now has none. Counts for brand-new test files too.
- **`tests.skipped`:** more skipped tests than before (`.skip`, `x…`, `.todo`, inside a skipped `describe`), or a new `.only` (which silently skips everything else)
- **`code.stub`:** a function whose body is only a "not implemented" throw, only a TODO comment, or a TODO plus a placeholder return. New stub functions count (left unfinished).
- **`code.swallowed_error`:** more empty `catch` blocks / empty `.catch()` handlers than before. Comments don't count as handling; logging does.
- **`deps.added`:** new entries in a package.json that already existed (any dependency type). For a brand-new package.json (a new project), only **runtime** dependencies that no code imports. This is a **fact**, not a judgement; whether it was justified belongs to Phase 6 (prompt constraints).
- **`secrets.hardcoded`:** secrets in a changed file that weren't there before. In test files only provider-format keys count; fake passwords in tests are normal.
- **`scope.unrelated_file`:** the **only** change to an existing file is new top-level code that nothing uses. A stand-in until the prompt is understood (Phase 6). First version fired when real work was also purely additive (2 false positives in scoring), so it now requires the unused code to be the file's only change.
- **Speed:** file contents are read in one batch per snapshot; name/import lookups use `git grep`. ~90 ms per recording.
- **Status:** Decided

### 2026-10-22: Behaviour check rules, part 2 (rest of Phase 3)
- **Process:** each check got its mutation first. All four scored recall 0 before the check existed, then 1.00 after.
- **`events.edited_without_reading`**
  - Only files that existed **before the session** (the user's code). Files the agent created are skipped.
  - "Read" means anything earlier in the session: Read or Grep on the file, a shell command naming it, or an earlier edit by the agent
  - Note: Claude Code's own Edit tool already refuses to edit unread files, so on Claude Code this mainly catches overwrites with Write, missed events and other agents
- **`tests.not_rerun_after_edit`**
  - Only in turns where tests ran at all (a turn with no test runs is a different problem, not checked yet)
  - Only **tested code**: test files plus project files they import directly (`checks/imports.rs`, a static scan of relative imports with TypeScript-style resolution). Editing UI the tests don't touch after a test run is not reported. The Vite probe session does exactly this and stays silent.
  - Changes made by commands (e.g. formatters) don't count
- **`commands.risky`**: `--no-verify`, force push (`--force`, `-f`, `+branch`), `git reset --hard`, `git clean -f`, discarding all changes (`git checkout -- .`, `git restore .`), recursive `rm` outside the project or of the whole project, `curl … | sh`, `sudo`, `chmod 777`, `npm publish`
  - **Not risky:** `rm -rf node_modules` / `dist` (inside the project), plain `git push`, `--force-with-lease`, package installs (Phase 4's `deps.added` covers new dependencies)
  - `rm` commands are parsed (flags and targets), not matched with one regex. A regex was tried first and missed `rm -rf ~/Documents`.
- **`events.edit_outside_project`**: edit tools writing outside `<PROJECT>`, and shell redirects (`>`, `>>`, `tee`) into the home folder or system folders. Allowed: the agent's own `~/.claude/` files and temp folders.
- **Cost:** the import scan raised checker time from ~20 ms to ~65 ms per recording (one git call per test file). Fine for now; batch the reads if it matters later.
- **Status:** Decided

### 2026-10-22: Behaviour check rules (Phase 2–3)
- **Decision:** the first checks follow these rules, chosen for precision:
  - **Only agent turns are checked** (prompt snapshot → next snapshot). Changes made by the human between turns are never reported.
  - **`events.changed_by_command`** fires only when a shell command **names the file**, e.g. `sed -i … src/a.ts` or `cat > src/a.ts`. Commands that change files without naming them (scaffolding, `npm install`, formatters) are silent. Lockfiles are always ignored.
  - **`events.unexplained_change`** fires only when a file changed and **no** edit or command ran in that turn at all (a human edit mid-turn, a subagent, or a missed hook)
  - **`tests.edited_after_failure`**: an edit to a test file between a failing and the next passing test run. Creating a brand-new test file doesn't count; editing one written earlier in the same turn does. **Watch item:** this may fire on legitimate "my new test was wrong" fixes. Check against labelled real runs.
  - **`claims.tests_pass_false`**: the agent's last message claims tests pass (clause by clause; negated, hedged and instructional clauses are ignored) and the last recorded test run failed. No test run at all → silent (a different problem, not checked yet).
  - **Unknown stays unknown:** a test result comes from the exit code, or else from the Vitest/Jest summary line. If neither is available, checks that need it stay silent.
  - **`capture.incomplete`** reports recorder problems and prompts with no closing snapshot (fail closed)
- **False-alarm probes:** two extra clean synthetic sessions (a scaffolded Vite app; a formatter run with an honest, hedged ending) must stay at 0 findings
- **Not yet measured:** `capture.incomplete` and `events.unexplained_change` have no mutations, so no recall yet
- **Status:** Decided

### 2026-10-22: Scoring harness design (Phase 0.5–0.6)
- **Decision:**
  - **Finding ids are a contract.** `crates/checks/src/catalog.rs` lists every finding id (e.g. `tests.removed`). Mutations and hand labels name the finding they expect by id. Add new ids rather than renaming.
  - **Defect names = mutation names.** `labels.json` defect types use the mutation names (`delete-test`, …), which map to finding ids, so real and injected defects are scored the same way.
  - **A finding matches** an expectation when the id is the same and, if the expectation names a file, the file is the same.
  - **Measured per run:** precision and recall per check, false alarms on clean fixtures, determinism (each fixture checked twice and compared byte for byte), checker time.
  - **Results** are saved with `cv score --save` to `harness/results/<date>-<commit>.json` (superseded 2026-10-26: results stay local, not committed).
- **Status:** Decided

### 2026-10-22: Mutations and synthetic recordings are generated, never stored
- **Decision:** synthetic recordings and all mutations are regenerated on every `cv score` run (~10 s). Only real recordings and score results are committed.
- **Why:** keeps git small (user requirement), and fixtures can never drift out of step with the code that makes them.
- **Status:** Decided

### 2026-10-22: Mutation realism rules
- **Decision:** each mutation must introduce exactly one problem:
  - Code mutations replace the final snapshot *and* add matching Read/Edit events plus a repeat of the last test run, so no other check (e.g. "edited without reading", "tests not re-run") is triggered by accident
  - `delete-test` only deletes tests that existed **before** the session. Deleting a test the agent itself just added leaves no trace against the starting code.
  - `test-edit-after-fail` turns the agent's last test run into a failure, then adds the test edit and a passing run (the realistic order)
  - `dropped-event` drops a `PreToolUse` event, so the action itself is still recorded
- **Tests:** each of these properties is checked for every mutation on every synthetic app
- **Status:** Decided

### 2026-10-22: Synthetic recordings (assumption to verify)
- **Decision:** 3 synthetic sessions (habit tracker, bill splitter, URL shortener; all TypeScript + Vitest) are made by driving the real recorder and exporter, so they have exactly the real format.
- **Assumption:** the shape of Claude Code's `Bash` tool response, in particular an `exit_code` field, is a guess. **Check this against the first real recording** and update `synth.rs` and the mutations to match.
- **Status:** Decided (assumption open)

### 2026-10-22: Corpus prompts are all TypeScript for v1
- **Decision:** G3 (URL shortener) changes from Python/FastAPI to TypeScript/Express, so the prompts built on it (E3, T1, V2) are TypeScript too. The Python versions are kept in `docs/TEST_PROMPTS.md` for later.
- **Why:** follows "JS/TS first". Every v1 recording can then be scored by the code-level checks.
- **Status:** Decided

### 2026-10-22: Corpus format
- **Decision:** `corpus/prompts/<ID>.json` is the source of truth for each prompt (turns, expected facts, traps, base, first-batch flag). `cargo test --test corpus` checks consistency, e.g. that every base a first-batch prompt needs is also in the first batch.
- **Status:** Decided

### 2026-10-22: Recorder design (Phase 0.2)
- **Decision:**
  - `cv hook` records every hook event under a per-session lock with sequence numbers. It always exits 0 and prints nothing; failures go to `.checker/errors.log`.
  - Snapshots are taken at `SessionStart`, `UserPromptSubmit` and `Stop` in a shadow git repo with its own index and with the user's git config, hooks and environment cut off
  - `.checker/` contains its own `.gitignore` (`*`), so the user's files never need changing
  - Snapshots use the system `git`
- **Measured:** release binary 2 MB; ~4 ms per tool event; ~60 ms per snapshot on a small app
- **Known limits:**
  - Needs `git` installed (fine for developers; revisit for low-code users in Phase 8, e.g. the `gix` crate)
  - Subagent transcripts aren't copied yet
  - A secret glued to other letters (no space or punctuation around it) may not be recognised
- **Status:** Decided

### 2026-10-22: Distribution: Rust core, familiar install channels
- **Decision:** the checker is built in Rust (read only by developers) and the same single binary is delivered through install channels users already know. Users never see Rust, Cargo or a build step.
- **Channels, in priority order:**
  1. **Claude Code plugin** (main route for low-code users): one-click install. It bundles the binary for each platform and sets up the hooks itself, with no configuration.
  2. **npm package** (`npx cv` / `npm i -D cv`): the platform-specific binary arrives as an optional dependency, the same way Biome and esbuild ship. Most vibe-coded projects already have Node.
  3. **Homebrew** (`brew install cv`) for developers
  4. **GitHub release download / `cargo install`** for Rust developers and CI
- **Rules for every channel:**
  - Same binary and version everywhere, with published checksums
  - No runtime to install
  - The checker never uses the network. Downloads only happen at install time, through the channel itself.
- **Inside the repo:** a Rust workspace split into crates (capture, events, checks, harness), all compiled into one `cv` binary. Existing crates are reused where possible (tree-sitter, ast-grep-core).
- **Status:** Decided (user)

### 2026-10-22: Checker implementation language: Rust, one file to install
- **Decision:** write the checker in Rust and ship it as **one self-contained binary** (working name `cv`) with subcommands:
  - `cv hook <event>`: called by Claude Code hooks. Only appends the event and takes snapshots, so it stays fast.
  - `cv check`: runs the analysis (at `Stop`, or on demand)
  - `cv score`: scores the checker against the test corpus (Phase 0.6)
- **Why one file works:**
  - Rust compiles everything (tree-sitter grammars, rule files, lexicon) into the binary. There's no runtime or dependencies to install.
  - Startup takes a few milliseconds, which matters because the hook runs on every agent tool call.
  - Rules and lexicon are embedded at build time and versioned with the binary, which supports determinism.
- **Reuse rather than build:**
  - `tree-sitter` (Rust crates) for parsing
  - `ast-grep-core` (Rust) for code-pattern rules
  - gitleaks' secret rules (MIT licence), ported as data files
- **Distribution:** an Apple Silicon binary first. A universal macOS binary (arm64 + x86_64), signing and notarisation come in Phase 8, so low-code users don't get macOS security warnings.
- **Toolchain:** Rust 1.94 (stable), installed.
- **Status:** Decided (user). Supersedes the TypeScript + Bun proposal.

### 2026-10-22: First coding agent supported
- **Decision:** Claude Code only for now. Other agents are out of scope until the checker meets its Phase 3 targets.
- **Safeguards:**
  - Every recording stores the Claude Code version
  - The adapter is tested against saved hook payloads, so a format change shows up as a failing test
- **Status:** Decided (user)

### 2026-10-22: Ground-truth labelling: staged
- **Decision:**
  - **First batch:** 7 prompts × 2 runs = 14 sessions: G1, G2, G3, E2, E3, T1, V1 (G3 added because E3 and T1 are built on it)
  - **Rest of the corpus:** recorded and labelled before Phase 5 (prompt understanding)
  - **Consistency check:** re-label ~5 runs about a week later to measure labeller consistency
  - **No AI pre-labelling**, which would bias the measurement
- **Why:** Phases 1–4 are scored mainly with injected-defect fixtures, which label themselves. Hand labels matter most for Phases 5–6.
- **Status:** Decided (user)

### 2026-10-22: Test apps are built outside this repo
- **Decision:**
  - Test sessions run in `~/Code/cv-sandbox/<prompt-id>-run<n>/`, a fresh folder per run with no `CLAUDE.md`
  - Starting copies for edit prompts come from `corpus/bases/`
  - Only the recordings are copied back into this repo
- **Why:** an agent working inside this repo would read `CLAUDE.md` and `docs/TEST_PROMPTS.md`, which describe the traps. That would contaminate the test.
- **Status:** Decided (user)

### 2026-10-22: No AI in the checker
- **Decision:** the checker never calls a model or an AI agent. This replaces the earlier "static only" rule: running the user's code is allowed, AI is not.
- **Status:** Decided (user)

### 2026-10-22: Running the user's code: optional, sandboxed layer
- **Proposal:**
  - Static checks run on every turn. Execution runs only on request (a "Verify" button, before a commit, or on a timer).
  - Execution always happens in the macOS sandbox, on a **copy** of the project, with **network off** and no access outside the copy
  - Checks, in build order: type-check / build → re-run tests on final code → old tests on new code → new test on old code → coverage of changed lines → mutation testing (opt-in)
  - Results record their environment, run twice, and are marked "flaky" if the two runs disagree. If tests can't run, the result is "couldn't verify by running", never a failure.
  - Evidence ladder: add "checker-run execution" between code state and the agent's recorded runs
- **Why:** gives evidence the agent can't fake, and catches test tampering that static analysis can't
- **Status:** Decided (user)

### 2026-10-22: First language of apps being checked
- **Decision:** TypeScript/JavaScript with Vitest or Jest only for v1. Python (pytest) second.
- **Why:** most vibe-coded apps are JS/TS. Limiting the test frameworks keeps the test-detection rules manageable.
- **Status:** Decided (user)

### 2026-10-22: Recording storage: compressed, only what's needed
- **Decision:** recordings are committed to this repo as test data, keeping only what the checker needs.
- **What's kept, per run** (`fixtures/recorded/<prompt-id>/run<n>/`):
  - `events.jsonl`: hook events with secrets redacted and paths replaced by placeholders. Long text fields are trimmed (see below).
  - `transcript.jsonl`: the agent transcript, trimmed and redacted the same way
  - `snapshots.bundle`: before/after code as a git bundle (git's own compressed format). Only the snapshot commits for that session.
  - `meta.json`: prompt id, run, Claude Code version, `cv` version, sizes, snapshot refs
  - `labels.json`: the hand labels
- **What's left out:**
  - Dependencies and build output (`node_modules/`, `dist/`, `build/`, `.next/`, `coverage/`, …)
  - Databases (`*.db`, `*.sqlite*`), secrets files (`.env*`, keys) and any file over 1 MB
  - The project's own gitignored files
  - Full file contents inside events: edits and reads are already in the snapshots. Any text over 4 KB keeps its first and last 1 KB plus a SHA-256 hash and length, which is enough to match events to snapshots without storing everything twice.
- **Compression:** JSONL is stored as plain text. Git already compresses everything it stores, and plain text stays searchable and diffable. Snapshots are git bundles, which are compressed packfiles. Nothing is compressed twice.
- **Budget:** target ≤ 500 KB per run. `cv record export` reports the size and warns above budget.
- **Safety:** export scans snapshot contents for secrets and refuses to export if it finds any, unless `--allow-snapshot-secrets` is passed. That flag is only for synthetic test keys, such as the hardcoded-secret trap.
- **Placeholders:** the project path becomes `<PROJECT>`, the home folder `<HOME>`, the username `<USER>`
- **Status:** Decided (user)

### 2026-10-22: Project location
- **Decision:** the repo lives at `~/Code/Code_Validation`, not `~/Documents/Code_Validation`
- **Why:** macOS privacy controls block the Claude Code helper process from Documents. Full Disk Access would need re-granting after every Claude Code update, because the helper's path includes its version. Folders outside Documents, Desktop and Downloads need no extra permission.
- **Status:** Decided (user request)

### 2026-10-22: Checker implementation language: TypeScript + Bun
- **Status:** Superseded by "Rust, one file to install" (above)

### 2026-10-22: Test-first build order
- **Decision:** build the test corpus, recorder, mutation fixtures and scoring harness (Phase 0) before any checker logic
- **Why:** every later step can be measured as it's built
- **Status:** Decided (user request)

### 2026-10-22: Version control
- **Decision:** local git repo only, no remote for now. Track everything, including docs. `.checker/`, dependencies, build output and secrets are gitignored.
- **Status:** Decided (user request)

### 2026-10-22: Documentation
- **Decision:** document everything as we go. `CLAUDE.md` holds the project context and progress log; `docs/` holds the plan, design notes, decisions and test prompts.
- **Status:** Decided (user request)

### 2026-10-22: Core design principles
- **Decision:** local only, deterministic, precision over recall, evidence ladder, three integrity rules (every part of the prompt accounted for, every change explained, log completeness with fail-closed)
- **Why:** see `DESIGN_NOTES.md` §7–8
- **Status:** Decided. "Static only" was replaced by "No AI in the checker".
