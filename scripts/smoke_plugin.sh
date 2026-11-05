#!/usr/bin/env bash
# Smoke test of the installed plugin, the way Claude Code runs it: the
# binary goes into a copy of plugin/ under its release name, and every call
# goes through the launcher (plugin/scripts/tattle), never the binary itself.
#
#   cargo build --release
#   scripts/smoke_plugin.sh target/release/tattle      # .exe on Windows
#
# One turn: Claude says "All tests pass." without running anything. The end
# of the turn must show that alert, and /tattle:report must list it.
# Runs on macOS, Linux and Windows (Git Bash). Exits non-zero on failure.
set -euo pipefail

bin=${1:?usage: scripts/smoke_plugin.sh <path to the tattle binary>}
repo=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# Native Windows programs need C:/… paths, not Git Bash's /tmp/….
native() { if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi; }
fail() { echo "smoke test FAILED: $*" >&2; exit 1; }

# The release names (.github/workflows/release.yml), chosen independently of
# the launcher so a mismatch between the two shows up here.
case "$(uname -s)" in
  Darwin) name=tattle-darwin-universal ;;
  Linux) case "$(uname -m)" in aarch64|arm64) name=tattle-linux-aarch64 ;; *) name=tattle-linux-x86_64 ;; esac ;;
  MINGW*|MSYS*|CYGWIN*) name=tattle-windows-x86_64.exe ;;
  *) fail "unknown system $(uname -s)" ;;
esac

cp -R "$repo/plugin" "$tmp/plugin"
rm -rf "$tmp/plugin/dist" && mkdir -p "$tmp/plugin/dist"
cp "$bin" "$tmp/plugin/dist/$name"
chmod +x "$tmp/plugin/dist/$name" "$tmp/plugin/scripts/tattle"
launcher="$tmp/plugin/scripts/tattle"

home="$tmp/home"
project="$home/Code/shop"
transcript="$home/.claude/projects/shop/session.jsonl"
mkdir -p "$project/src" "$(dirname "$transcript")"
printf '{"name":"shop","scripts":{"test":"node --test"}}\n' >"$project/package.json"
printf 'export const total = (xs) => xs.reduce((a, b) => a + b, 0);\n' >"$project/src/cart.js"
: >"$transcript"

export HOME="$home" USERPROFILE="$(native "$home")" CLAUDE_PLUGIN_ROOT="$(native "$tmp/plugin")" CLAUDE_PROJECT_DIR="$(native "$project")"
unset TATTLE_SOURCE TATTLE_REPORT_COMMAND TATTLE_MUTE TATTLE_TELL_CLAUDE CV_SOURCE 2>/dev/null || true

session=aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee
hook() { # event, extra JSON fields
  local payload out
  payload=$(printf '{"session_id":"%s","transcript_path":"%s","cwd":"%s","hook_event_name":"%s"%s}' \
    "$session" "$(native "$transcript")" "$(native "$project")" "$1" "${2:+,$2}")
  out=$(printf '%s' "$payload" | (cd "$project" && "$launcher" hook)) || fail "the $1 hook exited non-zero"
  printf '%s' "$out"
}

[ -z "$(hook SessionStart '"source":"startup"')" ] || fail "SessionStart printed something"
[ -z "$(hook UserPromptSubmit '"prompt":"add a discount option to total and tell me when the tests pass"')" ] || fail "UserPromptSubmit printed something"
message="Done. All tests pass."
printf '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"%s"}]}}\n' "$message" >>"$transcript"
stop=$(hook Stop "\"stop_hook_active\":false,\"last_assistant_message\":\"$message\"")
case "$stop" in
  *systemMessage*"never ran them"*) ;;
  *) fail "the end of the turn didn't show the alert; it printed: ${stop:-nothing}" ;;
esac

report=$(cd "$project" && "$launcher" report --project "$(native "$project")") || fail "/tattle:report exited non-zero"
case "$report" in
  *"tattle checked this session"*"never ran them"*) ;;
  *) fail "the report didn't list the alert; it printed: $report" ;;
esac

[ -d "$project/.checker" ] || fail "nothing was recorded in .checker/"
echo "smoke test passed ($name through the launcher)"
