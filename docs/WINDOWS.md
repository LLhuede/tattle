# Running on Windows

The project moved from a Mac to a Windows computer on 2026-10-29. This page covers setting it up there, and what works.

## What works on Windows

| | Windows |
|---|---|
| `cargo build`, `cargo clippy` | yes |
| `cargo test`, including the labelled real-session test (needs `fixtures/recorded/`) | yes |
| `tattle check`, `tattle score`, `tattle record export / restore` | yes |
| Recording new sessions with `tattle hook` | **yes**: the plugin in Claude Code's desktop app with Git Bash (2026-11-04, `docs/RESULTS.md` §3r) |
| `tattle verify` (running the project's code: plan #1–#4 execution parts, 4b probes) | **yes, with Docker Desktop running**; built and tested on Linux, not yet on Windows. See "Running code" below |
| `scripts/*.py` (recording runners, labels, big tests) | `release_numbers.py` yes; the others not yet checked on Windows |

Checked by cross-compiling for Windows and running the test suite under Wine with Git for Windows (2026-10-29): capture, checks, corpus, recorder and exec tests all pass; the harness tests (synthetic sessions and mutations) were still running at commit time. Since then it has run on a real Windows machine: the full test suite, and `tattle score` over all 163 labelled sessions (2026-11-05, `docs/RESULTS.md` §3s). CI runs the tests, the score and a plugin smoke test on Windows too.

## One-time setup

1. **Git for Windows** (https://git-scm.com/download/win). Claude Code needs it too.
2. **Visual Studio Build Tools**, with the "Desktop development with C++" workload. Rust uses its linker, and tree-sitter compiles C code.
3. **Rust** from https://rustup.rs (the default `x86_64-pc-windows-msvc` toolchain).
4. **Get the code.** In Git Bash, in a folder that isn't under `Documents`, `Desktop` or another OneDrive-synced folder (sync tools lock files mid-build and would upload recordings):
   ```bash
   git clone https://github.com/LLhuede/tattle && cd tattle
   ```
   Recordings (`fixtures/recorded/`) and big-test runs (`artifacts/`) are gitignored: copy them in if you have them.
5. **Keep it up to date** with `git pull`. It doesn't touch gitignored folders, so recordings stay.
6. **Build and test:**
   ```bash
   cargo build --release               # target\release\tattle.exe
   cargo test                          # now also runs the real-session test
   ```

## Platform details handled in the code

- **Paths:** `canonicalize()` returns `\\?\C:\…` on Windows, which git can't use; `capture::fsx::canonical` strips it.
- **Placeholders:** recorded Windows paths become `<PROJECT>/src/a.ts` (forward slashes), the form the checker reads. The project and home folders are replaced in every spelling: `C:\…`, `c:\…`, `C:/…`, Git Bash's `/c/…`.
- **Home and user name:** `USERPROFILE` and `USERNAME` on Windows (`HOME`/`USER` elsewhere), so they're redacted from recordings.
- **Deleting git folders:** git's object files are read-only, which blocks deletes on Windows; `capture::fsx::remove_dir_all` clears the flag.
- **`tattle record restore`** checks the snapshot out with git instead of piping to `tar`.
- **Hook command:** written with forward slashes (`'C:/…/tattle.exe' hook`), since Claude Code runs hooks through a shell (Git Bash on Windows).
- **Line endings:** `.gitattributes` keeps every text file LF on checkout, so the corpus bases and fixtures are byte-identical on every platform.

## Running code (`tattle verify`) on Windows

Docker (decided 2026-10-29). Install **Docker Desktop** and keep it running; `tattle` uses it when there's no macOS sandbox.
- **Built:** each command runs in a locked-down Linux container (`crates/exec/src/docker.rs`). The escape tests pass under Docker.
- **Dependencies:** the containers need Linux builds, so `tattle` keeps its own pool (`~/.cv/linux-deps`, one entry per lockfile). The first time, run with `--allow-install`: it downloads from the npm registry with install scripts off, then compiles native packages with the network off. Later runs reuse the pool offline.
  ```bash
  tattle verify <recording-folder> --allow-install --write
  ```
