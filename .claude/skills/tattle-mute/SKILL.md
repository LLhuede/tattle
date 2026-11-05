---
description: (development copy) Stop tattle showing one rule in this project, e.g. /tattle-mute deps.added. Each item in /tattle-report shows its rule.
argument-hint: <rule>
disable-model-invocation: true
allowed-tools: Bash(${CLAUDE_PROJECT_DIR}/plugin/scripts/tattle config *)
---

!`${CLAUDE_PROJECT_DIR}/plugin/scripts/tattle config mute "$ARGUMENTS"`

Show the person the text above exactly as written. Don't add anything else.
