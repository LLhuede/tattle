---
description: Show a muted tattle rule again in this project, e.g. /tattle:unmute deps.added.
argument-hint: <rule>
disable-model-invocation: true
allowed-tools: Bash(${CLAUDE_PLUGIN_ROOT}/scripts/tattle config *)
---

!`${CLAUDE_PLUGIN_ROOT}/scripts/tattle config unmute "$ARGUMENTS"`

Show the person the text above exactly as written. Don't add anything else.
