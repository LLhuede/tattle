---
description: Check your past Claude Code sessions on this computer. Shows how often "the tests pass" was backed by a test run, and which requests are worth a look. E.g. /tattle:audit 30 for the last 30 days.
argument-hint: [days]
disable-model-invocation: true
allowed-tools: Bash(${CLAUDE_PLUGIN_ROOT}/scripts/tattle audit *)
---

!`${CLAUDE_PLUGIN_ROOT}/scripts/tattle audit "$ARGUMENTS"`

Show the person the tattle audit above exactly as written, without adding to it or softening it. Don't start looking into any item unless they ask.
