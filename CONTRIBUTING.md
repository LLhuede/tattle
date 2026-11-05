# Contributing to tattle

The most useful contribution is a **hallucination tattle missed, or a false alarm it raised.** Open an issue with what you asked, what the agent said, and the `/tattle:report` output. Remove anything private first.

## Building and testing

You need Rust (stable).

```bash
cargo build --release
```

```bash
cargo test && cargo clippy --all-targets && target/release/tattle score
```

**This repo checks itself.** Opening it in Claude Code runs the project hooks in `.claude/settings.json`: your session is recorded into `.checker/` (local, gitignored, redacted, never sent anywhere), and the first session builds a frozen copy of tattle in the background (`scripts/dev_plugin.sh`). `/tattle-report` shows what it found. To turn this off, put `{"disableAllHooks": true}` in `.claude/settings.local.json` (which stays out of git).

`scripts/smoke_plugin.sh target/release/tattle` checks the plugin end to end through its launcher, as Claude Code runs it (CI runs it on all three systems).

`tattle score` reports precision and recall for every rule on injected defects (and on labelled real sessions, if you have recordings in `fixtures/recorded/`).

## How a rule gets in

Every rule is built test-first:

1. A defect injected into a recording (`crates/harness/src/mutate.rs`), plus a control that must stay silent.
2. The rule (`crates/checks/`), with unit tests.
3. A `tattle score` run showing its precision and recall, and no new false alarms.

Rule ids in `crates/checks/src/catalog.rs` are a contract: add new ones, don't rename old ones.

## What keeps tattle trustworthy

- **No AI in the checker.** Rules only; it never calls a model or the network.
- **Deterministic.** The same recording gives byte-identical output.
- **No secrets stored.** Redact before writing.
- **Measure on data the rules weren't tuned on**, and record a batch's numbers before fixing what it exposed.

More detail for maintainers and coding agents: [CLAUDE.md](CLAUDE.md), [docs/KNOWLEDGE.md](docs/KNOWLEDGE.md), [docs/DECISIONS.md](docs/DECISIONS.md).
