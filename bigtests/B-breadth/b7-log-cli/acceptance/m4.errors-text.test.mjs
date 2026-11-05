import { test } from "node:test";
import assert from "node:assert/strict";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { line, lines, run, runJson, tempFile } from "./_lib.mjs";

const sample = lines(line({ path: "/a", status: 200, rt: 10 }), line({ path: "/a", status: 404, rt: 30 }), line({ path: "/b", status: 200, rt: 20 }), "not a log line");

test("a missing file exits 1, names the file on stderr and prints nothing to stdout", () => {
  const missing = join(tmpdir(), "logstat-does-not-exist.log");
  const r = run(["--json", missing]);
  assert.equal(r.code, 1);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /^logstat: /);
  assert.ok(r.stderr.includes("logstat-does-not-exist.log"), r.stderr);
  const good = tempFile("good.log", sample);
  const mixed = run(["--json", good, join(tmpdir(), "logstat-nope.log")]);
  assert.equal(mixed.code, 1);
  assert.equal(mixed.stdout, "");
});

test("a directory given as an input file exits 1", () => {
  const r = run(["--json", tmpdir()]);
  assert.equal(r.code, 1);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /^logstat: /);
});

test("an unknown option exits 2 and the message names it", () => {
  const r = run(["--bogus-flag"], sample);
  assert.equal(r.code, 2);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /^logstat: /);
  assert.ok(r.stderr.includes("--bogus-flag"), r.stderr);
});

test("--top must be a positive integer and needs a value", () => {
  for (const args of [["--top", "0"], ["--top", "abc"], ["--top", "-3"], ["--top", "2.5"], ["--top"]]) {
    const r = run(args, sample);
    assert.equal(r.code, 2, args.join(" "));
    assert.equal(r.stdout, "", args.join(" "));
    assert.match(r.stderr, /^logstat: /, args.join(" "));
  }
});

test("--strict stops at the first malformed line with source and line number (stdin)", () => {
  const input = lines(line({}), "", "broken", line({}), "also broken");
  const r = run(["--json", "--strict"], input);
  assert.equal(r.code, 2);
  assert.equal(r.stdout, "");
  assert.ok(r.stderr.includes("<stdin>:3"), r.stderr);
  assert.match(r.stderr, /malformed log line/);
});

test("--strict names the file as given", () => {
  const f = tempFile("bad.log", lines(line({}), line({}), "oops"));
  const r = run(["--strict", f]);
  assert.equal(r.code, 2);
  assert.equal(r.stdout, "");
  assert.ok(r.stderr.includes(`${f}:3`), r.stderr);
});

test("--strict with only good lines (and blank lines) succeeds", () => {
  const r = run(["--json", "--strict"], lines(line({}), "", line({})));
  assert.equal(r.code, 0);
  assert.equal(JSON.parse(r.stdout).requests, 2);
});

test("text output is the default and has the documented lines", () => {
  const r = run([], sample);
  assert.equal(r.code, 0);
  assert.equal(r.stderr, "");
  const out = r.stdout.split("\n");
  assert.ok(out.includes("requests: 3"), r.stdout);
  assert.ok(out.includes("malformed: 1"), r.stdout);
  assert.ok(out.includes("top paths:"), r.stdout);
  assert.ok(out.includes("status codes:"), r.stdout);
  assert.ok(out.includes("response time (ms):"), r.stdout);
  assert.ok(out.includes("  2 /a"), r.stdout);
  assert.ok(out.includes("  1 /b"), r.stdout);
  assert.ok(out.includes("  200 2"), r.stdout);
  assert.ok(out.includes("  404 1"), r.stdout);
  assert.ok(out.includes("  count: 3"), r.stdout);
  assert.ok(out.includes("  max: 30"), r.stdout);
  assert.ok(out.includes("  p50: 20"), r.stdout);
  assert.throws(() => JSON.parse(r.stdout));
});

test("text output lists status codes in ascending order and top paths highest first", () => {
  const r = run([], lines(line({ status: 500, path: "/z" }), line({ status: 200, path: "/y" }), line({ status: 404, path: "/y" })));
  const out = r.stdout.split("\n");
  assert.ok(out.indexOf("  200 1") < out.indexOf("  404 1"));
  assert.ok(out.indexOf("  404 1") < out.indexOf("  500 1"));
  assert.ok(out.indexOf("  2 /y") < out.indexOf("  1 /z"));
});

test("text output says n/a when there are no response times", () => {
  const r = run([], lines(line({}), line({})));
  assert.ok(r.stdout.split("\n").includes("response time (ms): n/a"), r.stdout);
});

test("--help and -h print usage on stdout and exit 0", () => {
  for (const flag of ["--help", "-h"]) {
    const r = run([flag]);
    assert.equal(r.code, 0);
    assert.ok(r.stdout.includes("Usage"), r.stdout);
  }
});

test("--json output is one JSON document followed by a newline, with nothing on stderr", () => {
  const r = run(["--json"], sample);
  assert.equal(r.code, 0);
  assert.equal(r.stderr, "");
  assert.ok(r.stdout.endsWith("\n"));
  const out = JSON.parse(r.stdout);
  assert.equal(out.requests, 3);
  assert.equal(out.malformed, 1);
  assert.deepEqual(runJson([], sample), out);
});
