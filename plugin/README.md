# tattle

tattle catches your AI agent's hallucinations. It checks what Claude did against what you asked and what it said, and tells you in plain words when something doesn't add up. For example: Claude says the tests pass, but they failed the last time it ran them. Or you said "don't add packages" and it added one.

It runs on your computer. It never calls an AI model or the network, and what it records stays in a hidden `.checker` folder inside your project, which git ignores.

## Install

In Claude Code, type:

```
/plugin marketplace add LLhuede/tattle#plugin-dist
```

```
/plugin install tattle@tattle
```

That's all. Keep working as usual.

Works on macOS, Linux and Windows. On Windows, Claude Code must be using Git Bash (Git for Windows), which is its default.

## What you'll see

- **Nothing**, when everything checks out.
- **One line**, when Claude finishes a turn and tattle spotted something new. For example: *"⚠️ tattle: Claude said the tests pass, but they failed the last time it ran them. Type /tattle:report for details."*
- **`/tattle:report`** shows everything tattle noticed in this session, with the things worth a look first. Each item names its rule.
- **`/tattle:mute <rule>`** stops tattle showing that rule in this project; **`/tattle:unmute <rule>`** brings it back.
- **`/tattle:tell-claude on`** also sends new alerts to Claude at the end of a turn, so it corrects itself or says why tattle is wrong (at most once per turn). **`/tattle:tell-claude off`** stops it. Off by default.
- For every project: `TATTLE_MUTE=rule1,rule2` and `TATTLE_TELL_CLAUDE=1` in Claude Code's settings (`env`).

tattle checks what it can see: test runs, what Claude said, and the code changes. It can't tell whether the code does the right thing.

## Good to know

- tattle doesn't record when Claude Code is opened in your home folder rather than a project.
- It keeps the last 30 sessions per project and deletes anything older than 30 days.
- Secrets are replaced before anything is written. To delete what tattle recorded in a project, delete its `.checker` folder.
- To turn it off: `/plugin`, then the **Installed** tab, then select tattle and disable it.

More, including what tattle catches and how well: https://github.com/LLhuede/tattle
