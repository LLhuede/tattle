# App contract for big tests

Every spec in `bigtests/` tells the agent to build an app that follows this
contract, so the hidden acceptance tests can run against any implementation.
The agent sees the contract only as it appears in the spec's own words; it
never sees this repository or the hidden tests.

## All apps
- Node.js 18 (the version on the test machine). npm, with a `package.json`
  at the project root.
- The runner does `npm install` (or `npm ci` if a lockfile exists), then
  `npm run build` if a `build` script exists.
- Any language that runs on Node is fine (JS or TS). What the hidden tests
  touch must be reachable without the agent's own tooling (see below).

## HTTP servers
- `npm start` starts the server. It listens on `process.env.PORT`.
- All persistent data goes under `process.env.DATA_DIR` (a directory that
  exists and is empty at start). Nothing else is written outside it.
- `GET /health` returns `200` once the server is ready.
- JSON in, JSON out. Errors are JSON `{ "error": "<message>" }` with a 4xx
  or 5xx status.

## CLIs
- `package.json` has a `bin` entry. The hidden tests run
  `node <that file>` (after the build). It reads stdin and arguments, writes
  stdout/stderr, and exits 0 on success and non-zero on failure.
- Any state goes under `process.env.DATA_DIR` if set.

## Libraries
- `package.json` `main` (or `exports["."]`) points at a file Node can
  import after the build (built JS, CommonJS or ESM).

## Browser apps
- `npm run build` writes `dist/index.html`.
- Pure logic the spec names lives in a plain-JS ES module at the path the
  spec gives (for example `src/lib/streaks.js`), so the hidden tests can
  import it without a bundler.

## Hidden acceptance tests (for test authors)
- `node:test` + `node:assert/strict` + global `fetch`, with the helpers in
  `bigtests/common/harness.mjs`. No other dependencies.
- One file per milestone: `acceptance/<milestone>.<topic>.test.mjs`, where
  `<milestone>` is `m1`, `m2`, …, matching the suite's `milestones` list.
  A milestone's tests may assume earlier milestones are done.
- Test only what the spec states. Black-box: through HTTP, the CLI or the
  named module, never internal file names or framework details the spec
  doesn't fix.
- Each test file starts its own server (fresh `DATA_DIR`) and stops it.
