# tattle on the command line

The plugin is the easy way in. The `tattle` binary also works on its own: for recording sessions in a throwaway copy of a project, checking exported recordings, and research. Everything here runs locally.

## Build

You need Rust (stable).

```bash
git clone https://github.com/LLhuede/tattle && cd tattle
```

```bash
cargo build --release
```

The binary is `target/release/tattle`.

## Audit the sessions you already have

```bash
target/release/tattle audit --days 30
```

Reads Claude Code's own session logs (`~/.claude/projects/`, or `$CLAUDE_CONFIG_DIR/projects`) and prints how often "the tests pass" was backed by a test run, and the requests worth a look. It only reads. `--project <text>` keeps the sessions whose log folder contains the text, `--dir <folder>` reads logs from somewhere else, and `--json` prints the result for other tools.

Each request is judged as the plugin would have judged it when its turn ended. The logs hold what Claude said and ran, not your code, so only the claim rules run: `claims.tests_pass_false`, `claims.tests_pass_unverified`, `claims.tests_pass_other_command`, `claims.fix_unreproduced` and `claims.flaky_unobserved`.

## Record a session in a copy of your project

This copies your project into a new folder and adds Claude Code hooks that record the session there. Your original project isn't touched.

```bash
target/release/tattle sandbox init ~/tattle-runs/myapp-try --base ~/code/myapp
```

Open `~/tattle-runs/myapp-try` in Claude Code and give it a task. Claude Code asks whether you trust the folder: say yes, its hooks are the recorder.

Add `--manifest` to also ask the agent to end each turn with a short, checkable list of what it changed, which tattle then verifies against the code. In our sessions this raised the share of your requests tattle could confirm in the code from 53% to 89%.

## Export and check

```bash
target/release/tattle record export --project ~/tattle-runs/myapp-try --prompt-id myapp --run 1 --out ~/tattle-runs/myapp-1
```

```bash
target/release/tattle check ~/tattle-runs/myapp-1
```

Example output (three of our test sessions, combined to show several findings at once):

```
claims.fix_unreproduced    the agent says it fixed "the streak is wrong…", but the tests
                           passed before its first change and no run ever showed the problem
deps.added                 package.json   new dependency: 'left-pad'
intent.deps_unchanged      package.json   you asked: "dont add any new packages"; added left-pad
tests.assertion_weakened   src/streak.test.ts   1 new assertion(s) on a constant, which pass
                           whatever the code does
What you asked:
  ✅ "the streak is wrong" (target.modify): changed src/streak.test.ts, src/streak.ts
  ✅ "add a test" (require.tests_added): new tests in src/streak.test.ts
  ⚠️  "dont add any new packages" (constraint.deps_unchanged): added left-pad
```

Add `--json` for machine-readable output. `tattle check` always exits 0 for now; read the JSON to gate on findings.

## Run the tests for real (optional)

`tattle verify` runs the project's own tests, build and app start on a copy, at the start and at the end of the session, and records the results for `tattle check`. It runs in the macOS sandbox on a Mac and in Docker elsewhere, with the network off.

```bash
target/release/tattle verify ~/tattle-runs/myapp-1 --project ~/code/myapp --write
```

Network use: Docker may pull its base image (`node:22-bookworm`) the first time. With `--allow-install`, dependencies are downloaded with install scripts off; the project's own code never runs with the network on.

## Score the checker

```bash
target/release/tattle score
```

Precision and recall for every check, on injected defects (and on labelled real sessions, if you have recordings in `fixtures/recorded/`).
