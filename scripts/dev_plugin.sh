#!/bin/sh
# Builds tattle and installs a frozen copy where the plugin launcher looks for it
# (plugin/dist/tattle-<os>-<arch>, gitignored). This repo's own hooks
# (.claude/settings.json) record through that copy, so rebuilding
# target/release/tattle never disturbs a session that's recording.
#
#   scripts/dev_plugin.sh               rebuild and refresh the copy
#   scripts/dev_plugin.sh --if-missing  only if there's no copy yet (runs in
#                                       the background at session start, e.g.
#                                       in a fresh cloud container)
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
os=$(uname -s | tr '[:upper:]' '[:lower:]')
ext=""
case "$os" in
  mingw*|msys*|cygwin*|windows*) os=windows; ext=".exe" ;;
esac
arch=$(uname -m)
case "$arch" in
  arm64|aarch64) arch=aarch64 ;;
  x86_64|amd64) arch=x86_64 ;;
esac
dist="$root/plugin/dist"
dest="$dist/tattle-$os-$arch$ext"
if [ "${1:-}" = "--if-missing" ]; then
  [ -x "$dest" ] && exit 0
  command -v cargo >/dev/null 2>&1 || exit 0
  # Quiet: an async hook's output would be shown to Claude.
  mkdir -p "$root/target"
  exec >"$root/target/dev_plugin.log" 2>&1
fi
mkdir -p "$dist"
# One build at a time (two sessions starting together).
lock="$dist/.build.lock"
if ! mkdir "$lock" 2>/dev/null; then
  echo "another build is running ($lock)"
  exit 0
fi
trap 'rmdir "$lock"' EXIT
cargo build --release -p tattle --manifest-path "$root/Cargo.toml"
tmp="$dest.tmp.$$"
cp "$root/target/release/tattle$ext" "$tmp"
chmod +x "$tmp"
mv -f "$tmp" "$dest"   # atomic: running hooks keep the old file
echo "tattle for this repo's hooks: $dest"
