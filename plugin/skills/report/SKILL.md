---
description: Show what tattle found in this Claude Code session, in plain words.
disable-model-invocation: true
allowed-tools: Bash(${CLAUDE_PLUGIN_ROOT}/scripts/tattle report *)
---

!`${CLAUDE_PLUGIN_ROOT}/scripts/tattle report --session ${CLAUDE_SESSION_ID}`

Show the person the tattle report above exactly as written, without adding to it or softening it. Then, in one short sentence, offer to look into any item marked "Worth a look". Don't start fixing anything unless they ask.
