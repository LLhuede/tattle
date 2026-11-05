# Security

tattle runs on your machine, records Claude Code sessions into `.checker/` inside your project, and makes no network requests (see [Privacy and teams](README.md#privacy-and-teams)).

## Reporting a problem

Please report security problems privately, not in a public issue: use GitHub's **Report a vulnerability** button on the repository's **Security** tab. Include what you found, how to reproduce it, and which version you used (`tattle --version`).

Especially welcome:
- a secret that ends up unredacted in `.checker/` (transcripts and hook events are meant to be redacted; code snapshots are stored as they are, which is documented)
- anything that makes tattle reach the network
- a way out of the `tattle verify` sandbox (network, files outside the copy, secrets)
- a hook that blocks, slows or changes a Claude Code tool call

## What tattle runs

- The plugin's hooks run the `tattle` binary with your user's permissions on every Claude Code event. They always exit 0 and never block a tool call.
- `tattle verify` (opt-in, command line only) runs your project's own tests, build and app in the macOS sandbox or a locked-down Docker container with the network off.

Release binaries are built by the repository's `Plugin release` workflow from a tagged commit.
