import { test } from "node:test";
import assert from "node:assert/strict";
import { line, lines, run, runJson, tempFile } from "./_lib.mjs";

test("counts requests from stdin and ranks top paths by count", () => {
  const out = runJson([], lines(line({ path: "/a" }), line({ path: "/b" }), line({ path: "/a" }), line({ path: "/a" }), line({ path: "/b" }), line({ path: "/c" })));
  assert.equal(out.requests, 6);
  assert.equal(out.malformed, 0);
  assert.deepEqual(out.top_paths, [
    { path: "/a", count: 3 },
    { path: "/b", count: 2 },
    { path: "/c", count: 1 },
  ]);
});

test("reads a file argument", () => {
  const f = tempFile("a.log", lines(line({ path: "/x" }), line({ path: "/x" })));
  const out = runJson([f]);
  assert.equal(out.requests, 2);
  assert.deepEqual(out.top_paths, [{ path: "/x", count: 2 }]);
});

test("several files are counted together and '-' means stdin", () => {
  const f1 = tempFile("1.log", lines(line({ path: "/one" })));
  const f2 = tempFile("2.log", lines(line({ path: "/one" }), line({ path: "/two" })));
  const out = runJson([f1, f2]);
  assert.equal(out.requests, 3);
  assert.deepEqual(out.top_paths[0], { path: "/one", count: 2 });
  const mixed = runJson([f1, "-"], lines(line({ path: "/two" })));
  assert.equal(mixed.requests, 2);
});

test("combined-format lines (referer, user agent with spaces) parse like common ones", () => {
  const out = runJson([], lines(line({ path: "/a", combined: true }), line({ path: "/a" }), line({ path: "/b", combined: true, status: 404 })));
  assert.equal(out.requests, 3);
  assert.equal(out.malformed, 0);
  assert.deepEqual(out.top_paths, [
    { path: "/a", count: 2 },
    { path: "/b", count: 1 },
  ]);
});

test("the query string is not part of the path", () => {
  const out = runJson([], lines(line({ path: "/a?x=1" }), line({ path: "/a?y=2&z=3" }), line({ path: "/a" })));
  assert.deepEqual(out.top_paths, [{ path: "/a", count: 3 }]);
});

test("methods are ignored when counting paths", () => {
  const out = runJson([], lines(line({ method: "GET", path: "/api" }), line({ method: "POST", path: "/api" }), line({ method: "DELETE", path: "/api" })));
  assert.deepEqual(out.top_paths, [{ path: "/api", count: 3 }]);
});

test("status codes and classes are counted, only for codes that occurred", () => {
  const out = runJson([], lines(line({ status: 200 }), line({ status: 200 }), line({ status: 201 }), line({ status: 404 }), line({ status: 500 }), line({ status: 503 })));
  assert.deepEqual(out.status, { 200: 2, 201: 1, 404: 1, 500: 1, 503: 1 });
  assert.deepEqual(out.status_classes, { "2xx": 3, "4xx": 1, "5xx": 2 });
});

test("--top limits the list, ties are broken by path ascending, and the default is 10", () => {
  const ls = [];
  for (const p of ["/d", "/c", "/b", "/a"]) ls.push(line({ path: p }));
  const out = runJson(["--top", "3"], lines(...ls));
  assert.deepEqual(out.top_paths.map((t) => t.path), ["/a", "/b", "/c"]);
  const many = [];
  for (let i = 0; i < 12; i++) many.push(line({ path: `/p${String(i).padStart(2, "0")}` }));
  const def = runJson([], lines(...many));
  assert.equal(def.top_paths.length, 10);
  assert.equal(def.requests, 12);
});

test("--top=N works too", () => {
  const out = runJson(["--top=1"], lines(line({ path: "/a" }), line({ path: "/a" }), line({ path: "/b" })));
  assert.deepEqual(out.top_paths, [{ path: "/a", count: 2 }]);
});

test("malformed lines are skipped and counted; blank lines are ignored; no stderr", () => {
  const input = lines(line({ path: "/ok" }), "this is not a log line", "", line({ path: "/ok" }), '127.0.0.1 - - [bad date] "GET / HTTP/1.1" 200 1', line({ status: 200, bytes: "-" }).replace(" 200 -", " 20 -"), "   ");
  const r = run(["--json"], input);
  assert.equal(r.code, 0);
  assert.equal(r.stderr, "");
  const out = JSON.parse(r.stdout);
  assert.equal(out.requests, 2);
  assert.equal(out.malformed, 3);
});

test("'-' fields (ident, user, bytes) are fine; a request that isn't METHOD target HTTP/x is malformed", () => {
  const good = '10.0.0.1 - alice [10/Oct/2023:13:55:36 +0000] "GET /ok HTTP/1.0" 304 -';
  const bad1 = '10.0.0.1 - - [10/Oct/2023:13:55:36 +0000] "-" 408 -';
  const bad2 = '10.0.0.1 - - [10/Oct/2023:13:55:36 +0000] "GET /nohttp" 200 5';
  const out = runJson([], lines(good, bad1, bad2));
  assert.equal(out.requests, 1);
  assert.equal(out.malformed, 2);
  assert.deepEqual(out.status, { 304: 1 });
});

test("empty input gives a zeroed report and exit code 0", () => {
  const r = run(["--json"], "");
  assert.equal(r.code, 0);
  assert.deepEqual(JSON.parse(r.stdout), { requests: 0, malformed: 0, top_paths: [], status: {}, status_classes: {}, response_time_ms: null });
});
