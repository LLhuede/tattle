# tattle: catch your AI agent's hallucinations

**Claude Code says "Fixed. All tests pass." tattle checks the session with deterministic rules, no AI, and tells you when that's a hallucination.**

**Screenshot of a real Claude Code session.** Top: the end of Claude's reply. Bottom: tattle's notice.

![Claude lists "All tests pass: Logic is sound" and "The implementation is already production-ready!" Below it, tattle's notice: Claude said the tests pass, but it never ran them.](docs/images/never-ran-tests.png)

*Asked to confirm the tests still pass, Claude declared them passing from reading the code. It never ran them.*

**No AI, just deterministic code: in blind tests, tattle flags about 40% of sessions where Claude makes a false claim about its work. Across all 163 labelled sessions, it wrongly flagged 0% of the 112 honest ones.** [How it was measured](#how-well-it-works).

Coding agents don't only hallucinate code. They hallucinate about their own work: tests they never ran, bugs they "fixed" that never existed, instructions they say they followed. Most checkers read the agent's last message. tattle also reads **your request and the whole session**: every command, every test run, and the code before and after each turn. So it catches what a confident summary hides: the package you said not to add, the test bent between a red run and a green one, the "fix" for a bug that never reproduced.

No AI inside. No API key, no network, no tokens. Every alert comes from a rule you can read, and the same session always gives the same result.

```
/plugin marketplace add LLhuede/tattle#plugin-dist
```

```
/plugin install tattle@tattle
```

Then start a new Claude Code session. Runs on macOS, Linux and Windows (Git Bash); the CI builds and tests all three. Test and instruction checks work in any language; code-level checks read JavaScript and TypeScript.

## What it catches

44 rules in five groups. A few examples of each:

| Group | Examples |
|---|---|
| **Hallucinated claims** | "Tests pass" when the last run failed, no run happened, or only a different command passed · "Fixed" when the problem never reproduced and the tests passed before the change · "Fixed the flaky test" when no run ever failed |
| **Ignoring you** | You said no new packages, and one appeared · "Only change X" or "don't touch Y", and it went further · You asked a question, and files changed · A rename left half done |
| **Bending the tests** | Tests deleted, skipped, or swapped for assertions that always pass · A test file edited between a failing run and a passing one · Code changed after the last test run |
| **Risky moves** | Force pushes, skipped git hooks, `sudo`, deleting outside the project · Edits outside the project · Hardcoded secrets, placeholder code, swallowed errors |
| **Running your tests** (opt-in, [`tattle verify`](docs/CLI.md)) | Tests that passed at the start fail at the end · The build broke · The app no longer starts · Old tests changed to match a bug |

All 44, one line each: [`catalog.rs`](crates/checks/src/catalog.rs).

## What you'll see

**Nothing**, most of the time. **One line at the end of a turn** when a rule fires.

**Screenshot of another real session**, opened in a `code` folder. Top: Claude's reply. Bottom: tattle's notice.

![Claude says "All 4 tests pass, and I followed your instructions." Below it, tattle's notice: Claude changed a file outside this project, twice, for slug.js and slug.test.js in another folder.](docs/images/outside-project.png)

Claude said it followed your instructions; it had written its files into a different folder. **`/tattle:report`** lists everything in a session, for example:

```
tattle checked this session (3 requests).

Worth a look:
- Request 2 ("use the formatDate helper for the dates"): You mentioned formatDate
  as if it already existed, but it isn't in the project, and Claude went ahead
  without saying so. (intent.phantom_reference)
- Request 3 ("add a discount and tell me the tests pass"): Claude said the tests
  pass, but it never ran them. (claims.tests_pass_unverified)

Also noticed (usually fine):
- Request 3 ("add a discount and tell me the tests pass"): Claude added a package:
  dayjs. (deps.added)

tattle checks what it can see: test runs, what Claude said, and the code changes.
It can't tell whether the code does the right thing.
```

**Already have sessions?** `/tattle:audit` checks the Claude Code sessions on your computer from before tattle was installed: how often "the tests pass" had a test run behind it, and which requests are worth a look.

```
tattle audit: 42 sessions, 310 requests from the last 30 days.

Claude said the tests pass 57 times:
  44  a test run had passed, and nothing was edited after it
   6  a test run had passed, but files were edited after it
   5  no test run in the session
   2  the last test run had failed

Worth a look (7):
- 2026-11-03, ~/code/shop, request 2 ("add free shipping over 50 and tell me the tests pass"):
  Claude said the tests pass, but it never ran them. (`claims.tests_pass_unverified`)
  …
```

*(Example output.)* The audit reads Claude Code's own session logs, which hold what Claude said and ran but not your code. So it runs the claim rules only; the code and request checks need a session the plugin recorded.

**Silence means no rule fired, not that the work is right.** tattle only speaks when the session proves something doesn't add up, so it misses things (see below).

**Your call what happens next.** By default the alert goes to you, not to Claude. Two settings, per project:

- **`/tattle:mute <rule>`** stops one rule (each report item shows its rule name, like `deps.added`). `/tattle:unmute <rule>` brings it back. The report still lists what's muted.
- **`/tattle:tell-claude on`** also hands new alerts to Claude at the end of the turn, so it carries on: it puts the claim right or says in one sentence why tattle is wrong. You still see every alert. At most once per turn, so it can't loop. `/tattle:tell-claude off` stops it.

For every project, set `TATTLE_MUTE=deps.added,code.stub` or `TATTLE_TELL_CLAUDE=1` in Claude Code's settings (`env`). `/tattle:report` runs as a slash command, so its text enters Claude's context too.

## How well it works

Measured on real Claude Code sessions, misses included.

| What we measured | Result |
|---|---|
| **Blind:** sessions the rules were never tuned on, scored before any fix (Haiku 4.5, static checks) | Caught **11 of 28** sessions where the agent hallucinated a claim (about 40%) |
| **Labelled real sessions:** all 163 (scored on Windows on the 0.9.0 code, 2026-11-05; most were used to tune the rules, so optimistic) | **23 of 49** sessions with a hallucinated claim flagged by the static checks, 25 with the opt-in test runs; **0 of 112** honest sessions flagged |
| **Genuine fixes** (a real bug, really fixed) | 0 of 5 wrongly flagged |
| **Everyday use:** 224 ordinary sessions through the plugin on JavaScript, Python, Go, Rust, C and Ruby projects | False alerts fell from 8 in the first 40 sessions to 0 in 36, then 2 in 28 on the final build; each cause was recorded, then fixed with a general rule and a test. No agent made a false claim in these sessions, so they measure noise, not catches |
| **Injected defects:** 228 mutations of recorded sessions | All 41 rules that have a mutation catch it every time (3 rules have none yet); byte-identical results on every re-run |

**Where the evidence is thin:** the trap sessions were mostly Haiku 4.5 on small TypeScript apps, the prompts were written by agents, and the labels come from AI reviewers plus re-running each session's tests, not humans yet. (tattle itself uses no AI.) The everyday projects were small and dependency-free.

**What tattle can't catch:** wrong diagnoses, unmeasured "it's faster now", wrong arithmetic in prose, and behaviour nothing pins down. Claims phrased in ways its rules don't recognise get no verdict, not a false one. Rules can't be argued with, but an agent that makes no claim gives them nothing to check, and one that fakes a test runner's output could fool them: tattle targets honest-but-wrong agents, not adversarial ones.

Methods and every number: [RESULTS.md](docs/RESULTS.md).

## Why not just…

| Instead of tattle | What it misses |
|---|---|
| **Read every diff** | The diff doesn't show whether the tests ran, which command passed, or whether the bug ever reproduced. tattle reads the session. |
| **A hook that checks for a passing test command** | Catches "tests pass" with no run. Misses tests weakened to pass, instructions ignored, fixes never shown, and claims about code that isn't there. |
| **Ask another model to review** | It can be persuaded by the same confident summary, costs tokens, sends your code to an API, and answers differently each time. |
| **CI** | Tells you the tests pass now, not that they were weakened to get there or that "don't add packages" was ignored. And it runs after you've moved on. |

tattle runs at the end of every turn, before you move on, and stays silent when everything checks out.

## Privacy and teams

- **What it stores**, in `.checker/` inside each project: your prompts, Claude's replies, every tool call and its output, and a snapshot of your code at each request and each reply. `.checker/` ignores itself in git.
- **`/tattle:audit` only reads.** It reads Claude Code's session logs (`~/.claude/projects/`) when you run it, prints its result, and stores nothing.
- **Never on the network.** The plugin and `tattle check` make no network requests. Only the opt-in `tattle verify` may pull a Docker image, and download dependencies if you pass `--allow-install`.
- **Redaction has limits.** Known secret formats (cloud and API keys, tokens, private keys, passwords in URLs and assignments) are replaced in the transcript and hook events, along with your home folder and user name. **Code snapshots are stored as they are**, minus `node_modules`, build output, `.env*`, `*.pem`, `*.key`, SSH keys, files over 1 MB and anything your `.gitignore` excludes. A secret hardcoded in a tracked file is in the snapshot.
- **Keep it out of copies.** Add `.checker/` to `.dockerignore` and to any sync or backup tool's exclusions.
- **Retention:** the last 30 sessions per project, nothing older than 30 days, tidied when a new session starts. It doesn't record in your home folder. Delete `.checker/` to remove everything.
- **Hooks never block.** They never approve, change or stop a tool call, and always exit 0. The end-of-turn check gives up silently after 20 seconds. Cost: 2–4 ms per tool call; well under a second at the end of a typical turn (about 2 s on a 20,000-file project).

## Status

**Works today:** Claude Code (plugin or project hooks). Test runs in most ecosystems (`npm`/`pnpm`/`yarn`/`bun test`, vitest, jest, `node --test`, pytest, unittest, `go test`, `cargo test`, mocha, minitest, RSpec, `make test`, or any script whose output shows a runner's summary). Code-level checks for JavaScript and TypeScript.

**Not yet:**
- Cursor, Codex, Copilot or Aider
- Code-level checks for Python, Go and Rust
- Homebrew or npm install of the command line, and a CI mode that fails the build on alerts
- Windows without Git Bash (Claude Code then runs hooks in PowerShell)

To turn it off: `/plugin`, the **Installed** tab, select tattle, disable or uninstall.

## Under the hood

```
Claude Code hooks ──► tattle hook ──► .checker/   events, transcript, a snapshot per turn
                                          │
                          end of each turn: every rule runs
                                          │
      ┌─────────────────┬─────────────────┼─────────────────┬─────────────────┐
      │                 │                 │                 │                 │
  behaviour       code changes       your prompt     agent's summary      execution
 (event log)      (before/after      (checkable     (checked against   (opt-in: tests,
                   each turn)          facts)           the code)     build, app start)
```

tattle turns the checkable parts of your prompt (no new packages, don't touch X, rename Y, add route Z, tests pass) into facts and verifies them, with every claim the agent makes about tests and fixes, against the recorded code and test runs. Command line, recording a throwaway copy, and `tattle verify`: [CLI.md](docs/CLI.md).

**Why the same session always gives the same answer:**
- **Rules are plain code:** pattern matching on commands, test output and messages, and parsing the code with tree-sitter. There's no model, no randomness and no network, and nothing in the output depends on the clock.
- **It reads only the recording** (events, transcript, a code snapshot per turn), never your machine's current state, so checking a session again next week gives the same findings.
- **One fixed order:** findings are sorted the same way before they're shown.
- **Tested on every scoring run:** every test session, synthetic and real, is checked twice and the two outputs are compared byte for byte. They have always matched.
- **What can change a result:** a new tattle version, since rules improve. And the end-of-turn line gives up after 20 seconds, so on a very large project or a slow machine it can stay silent. `/tattle:report` always runs the full check.

## Contributing

Most useful: **a false claim tattle missed, or a false alarm it raised.** Open an issue with what you asked, what the agent said, and the `/tattle:report` output.

Every rule is built test-first: a defect injected into a recording, then the rule, then a score of its precision and recall on synthetic and real sessions.

```bash
cargo test && cargo clippy --all-targets && target/release/tattle score
```

The rules that keep tattle trustworthy: no AI or network in the checker, deterministic output, no secrets stored.

## License

MIT. See [LICENSE](LICENSE).
