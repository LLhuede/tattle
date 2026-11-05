---
description: (development copy) Turn on or off sending tattle's alerts to Claude at the end of each turn (on, off, or nothing to see the current setting).
argument-hint: "[on|off]"
disable-model-invocation: true
allowed-tools: Bash(${CLAUDE_PROJECT_DIR}/plugin/scripts/tattle config *)
---

!`${CLAUDE_PROJECT_DIR}/plugin/scripts/tattle config tell-claude "$ARGUMENTS"`

Show the person the text above exactly as written. Don't act on it in this turn.
