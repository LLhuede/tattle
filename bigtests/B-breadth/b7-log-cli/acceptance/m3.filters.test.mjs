import { test } from "node:test";
import assert from "node:assert/strict";
import { line, lines, run, runJson } from "./_lib.mjs";

// Times in UTC (tz +0000) at 10:00:00, 10:30:00, 11:00:00 on 2023-10-10.
const at = (h, mi) => ({ d: 10, mon: 10, y: 2023, h, mi, s: 0, tz: "+0000" });
const three = lines(line({ at: at(10, 0), path: "/early" }), line({ at: at(10, 30), path: "/mid" }), line({ at: at(11, 0), path: "/late" }));

test("--since is inclusive", () => {
  const out = runJson(["--since", "2023-10-10T10:30:00Z"], three);
  assert.deepEqual(out.top_paths.map((t) => t.path).sort(), ["/late", "/mid"]);
  assert.equal(out.requests, 2);
});

test("--until is exclusive", () => {
  const out = runJson(["--until", "2023-10-10T10:30:00Z"], three);
  assert.deepEqual(out.top_paths.map((t) => t.path), ["/early"]);
});

test("since and until together make a half-open window", () => {
  const out = runJson(["--since=2023-10-10T10:00:00Z", "--until=2023-10-10T11:00:00Z"], three);
  assert.equal(out.requests, 2);
  assert.deepEqual(out.top_paths.map((t) => t.path).sort(), ["/early", "/mid"]);
});

test("log timestamps with an offset are compared as instants", () => {
  // 13:55:36 -0700 is 20:55:36Z
  const input = lines(line({ at: { h: 13, mi: 55, s: 36, tz: "-0700" }, path: "/pdt" }), line({ at: { h: 20, mi: 55, s: 35, tz: "+0000" }, path: "/before" }), line({ at: { h: 22, mi: 55, s: 36, tz: "+0200" }, path: "/cest" }));
  const since = runJson(["--since", "2023-10-10T20:55:36Z"], input);
  assert.deepEqual(since.top_paths.map((t) => t.path).sort(), ["/cest", "/pdt"]);
  const until = runJson(["--until", "2023-10-10T20:55:36Z"], input);
  assert.deepEqual(until.top_paths.map((t) => t.path), ["/before"]);
});

test("offsets in the option value are honoured", () => {
  const out = runJson(["--since", "2023-10-10T12:30:00+02:00"], three);
  assert.equal(out.requests, 2);
  const none = runJson(["--since", "2023-10-10T12:30:01+02:00"], three);
  assert.equal(none.requests, 1);
});

test("a plain date means 00:00:00 UTC of that day", () => {
  const input = lines(line({ at: { d: 9, h: 23, mi: 59, s: 59, tz: "+0000" }, path: "/nine" }), line({ at: { d: 10, h: 0, mi: 0, s: 0, tz: "+0000" }, path: "/ten" }), line({ at: { d: 11, h: 0, mi: 0, s: 0, tz: "+0000" }, path: "/eleven" }));
  const out = runJson(["--since", "2023-10-10", "--until", "2023-10-11"], input);
  assert.deepEqual(out.top_paths, [{ path: "/ten", count: 1 }]);
});

test("--status takes exact codes, classes and comma lists", () => {
  const input = lines(line({ status: 200 }), line({ status: 404 }), line({ status: 404 }), line({ status: 500 }), line({ status: 503 }), line({ status: 301 }));
  assert.deepEqual(runJson(["--status", "404"], input).status, { 404: 2 });
  assert.deepEqual(runJson(["--status", "5xx"], input).status, { 500: 1, 503: 1 });
  const both = runJson(["--status", "404,500,3xx"], input);
  assert.deepEqual(both.status, { 301: 1, 404: 2, 500: 1 });
  assert.equal(both.requests, 4);
});

test("--method is case-insensitive and takes a list", () => {
  const input = lines(line({ method: "GET", path: "/g" }), line({ method: "POST", path: "/p" }), line({ method: "PUT", path: "/u" }));
  assert.deepEqual(runJson(["--method", "post"], input).top_paths, [{ path: "/p", count: 1 }]);
  assert.equal(runJson(["--method", "get,PUT"], input).requests, 2);
});

test("filters combine and apply to every statistic", () => {
  const input = lines(
    line({ at: at(10, 0), method: "GET", path: "/a", status: 200, rt: 10 }),
    line({ at: at(10, 30), method: "GET", path: "/a", status: 500, rt: 200 }),
    line({ at: at(10, 45), method: "GET", path: "/b", status: 500, rt: 400 }),
    line({ at: at(10, 50), method: "POST", path: "/b", status: 500, rt: 900 }),
    line({ at: at(11, 30), method: "GET", path: "/c", status: 500, rt: 5000 }),
  );
  const out = runJson(["--since", "2023-10-10T10:15:00Z", "--until", "2023-10-10T11:00:00Z", "--status", "5xx", "--method", "get"], input);
  assert.equal(out.requests, 2);
  assert.deepEqual(out.status, { 500: 2 });
  assert.deepEqual(out.top_paths.map((t) => t.path), ["/a", "/b"]);
  assert.equal(out.response_time_ms.count, 2);
  assert.equal(out.response_time_ms.max, 400);
});

test("filters don't change the malformed count", () => {
  const out = runJson(["--status", "404"], lines(line({ status: 200 }), "garbage", line({ status: 404 }), "more garbage"));
  assert.equal(out.requests, 1);
  assert.equal(out.malformed, 2);
});

test("a window that matches nothing is still a success with a zeroed report", () => {
  const r = run(["--json", "--since", "2030-01-01"], three);
  assert.equal(r.code, 0);
  const out = JSON.parse(r.stdout);
  assert.equal(out.requests, 0);
  assert.deepEqual(out.top_paths, []);
  assert.equal(out.response_time_ms, null);
});

test("bad timestamps, a reversed window and bad filter values are usage errors", () => {
  const cases = [
    ["--since", "yesterday"],
    ["--until", "2023-13-45"],
    ["--since", "2023-10-10T10:00:00"],
    ["--since", "2023-10-11", "--until", "2023-10-10"],
    ["--status", "abc"],
    ["--status", "700"],
    ["--status", "6xx"],
  ];
  for (const args of cases) {
    const r = run(["--json", ...args], three);
    assert.equal(r.code, 2, args.join(" "));
    assert.equal(r.stdout, "", args.join(" "));
    assert.match(r.stderr, /^logstat: /, args.join(" "));
  }
});
