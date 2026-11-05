#!/bin/sh
# Builds the tattle plugin for users and puts it on the `plugin-dist` branch:
# one commit holding only the marketplace file, the plugin and its binaries
# (main never carries binaries). Users then install it with:
#
#   /plugin marketplace add LLhuede/tattle#plugin-dist
#   /plugin install tattle@tattle
#
# Run it on each system (or let .github/workflows/release.yml build them
# all). On a Mac it includes macOS binaries (both chips when the targets are
# installed: rustup target add aarch64-apple-darwin x86_64-apple-darwin).
# Binaries already in plugin/dist from other machines (e.g. a Linux build)
# are included too.
#
#   scripts/release_plugin.sh             build the branch locally
#   scripts/release_plugin.sh --push      ...and push it (replaces the branch)
#   scripts/release_plugin.sh --no-build  only package the binaries already
#                                         in plugin/dist (CI builds them)
set -eu
push=no
build=yes
for arg in "$@"; do
  case "$arg" in
    --push) push=yes ;;
    --no-build) build=no ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
dist="$root/plugin/dist"
mkdir -p "$dist"

case "$build:$(uname -s)" in
  no:*) ;;
  *:Darwin)
    built=""
    for t in aarch64-apple-darwin x86_64-apple-darwin; do
      if rustup target list --installed 2>/dev/null | grep -qx "$t"; then
        cargo build --release -p tattle --target "$t"
        built="$built target/$t/release/tattle"
      fi
    done
    if [ "$(echo $built | wc -w)" -eq 2 ]; then
      lipo -create -output "$dist/tattle-darwin-universal" $built
    else
      cargo build --release -p tattle
      cp target/release/tattle "$dist/tattle-darwin-$(uname -m | sed 's/arm64/aarch64/')"
    fi
    ;;
  *:Linux)
    cargo build --release -p tattle
    cp target/release/tattle "$dist/tattle-linux-$(uname -m | sed 's/amd64/x86_64/;s/arm64/aarch64/')"
    # A Windows build too, when the cross target is installed
    # (rustup target add x86_64-pc-windows-gnu; apt install mingw-w64).
    if rustup target list --installed 2>/dev/null | grep -qx x86_64-pc-windows-gnu; then
      cargo build --release -p tattle --target x86_64-pc-windows-gnu
      cp target/x86_64-pc-windows-gnu/release/tattle.exe "$dist/tattle-windows-x86_64.exe"
    fi
    ;;
  *:MINGW*|*:MSYS*|*:CYGWIN*)
    # Git Bash on Windows.
    cargo build --release -p tattle
    cp target/release/tattle.exe "$dist/tattle-windows-x86_64.exe"
    ;;
esac

# A release tag (v1.0.0) names the version; otherwise version-commit.
version=$(git describe --tags --exact-match --match 'v[0-9]*' 2>/dev/null | sed 's/^v//') || true
[ -n "$version" ] || version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)-$(git rev-parse --short HEAD)"
work=$(mktemp -d)
trap 'git worktree remove --force "$work" 2>/dev/null || rm -rf "$work"' EXIT
git worktree add --detach "$work" >/dev/null
cd "$work"
# A temporary branch from an earlier run would make --orphan fail.
git branch -D plugin-dist-new >/dev/null 2>&1 || true
git checkout --orphan plugin-dist-new >/dev/null 2>&1
git rm -rf -q . >/dev/null 2>&1 || true
rm -rf ./*
mkdir -p .claude-plugin
cp -R "$root/plugin" ./plugin
rm -f plugin/dist/.build.lock
# Only this release's binaries (not leftovers such as the pre-rename cv-*).
find plugin/dist -type f ! -name 'tattle-*' -exec rm -f {} +
# Stamp the version so installed copies know there's an update.
sed "s/\"version\": \"[^\"]*\"/\"version\": \"$version\"/" "$root/plugin/.claude-plugin/plugin.json" > plugin/.claude-plugin/plugin.json
cat > .claude-plugin/marketplace.json <<JSON
{
  "name": "tattle",
  "owner": { "name": "LLhuede" },
  "description": "tattle: catch your AI agent's hallucinations.",
  "plugins": [
    { "name": "tattle", "source": "./plugin", "description": "Catches your AI agent's hallucinations, in plain words." }
  ]
}
JSON
git add -A
git add -f plugin/dist
git -c user.name="tattle release" -c user.email="noreply@localhost" commit -q -m "tattle plugin $version"
git branch -f plugin-dist HEAD
git checkout -q --detach && git branch -D plugin-dist-new >/dev/null 2>&1 || true
echo "plugin-dist is now tattle plugin $version with:"
ls plugin/dist
if [ "$push" = yes ]; then
  git push -f origin plugin-dist
fi
