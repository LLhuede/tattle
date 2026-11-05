---
description: Show what tattle found in this session, in plain words (development copy of the plugin's /tattle:report).
disable-model-invocation: true
allowed-tools: Bash(${CLAUDE_PROJECT_DIR}/plugin/scripts/tattle report *)
---

!`${CLAUDE_PROJECT_DIR}/plugin/scripts/tattle report --session ${CLAUDE_SESSION_ID}`

Show the person the tattle report above exactly as written, without adding to it or softening it. Then, in one short sentence, offer to look into any item marked "Worth a look". Don't start fixing anything unless they ask.
