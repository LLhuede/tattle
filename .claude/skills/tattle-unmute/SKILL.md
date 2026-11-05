---
description: (development copy) Show a muted tattle rule again in this project, e.g. /tattle-unmute deps.added.
argument-hint: <rule>
disable-model-invocation: true
allowed-tools: Bash(${CLAUDE_PROJECT_DIR}/plugin/scripts/tattle config *)
---

!`${CLAUDE_PROJECT_DIR}/plugin/scripts/tattle config unmute "$ARGUMENTS"`

Show the person the text above exactly as written. Don't add anything else.
