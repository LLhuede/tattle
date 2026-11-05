import { test } from "node:test";
import assert from "node:assert/strict";
import { line, lines, run, runJson } from "./_lib.mjs";

const withTimes = (vals, extra = {}) => lines(...vals.map((rt) => line({ rt, ...extra })));

test("count, min and max over lines that have a response time (combined format)", () => {
  const out = runJson([], withTimes([30, 10, 20], { combined: true }));
  assert.equal(out.response_time_ms.count, 3);
  assert.equal(out.response_time_ms.min, 10);
  assert.equal(out.response_time_ms.max, 30);
});

test("nearest-rank percentiles on ten values", () => {
  const out = runJson([], withTimes([10, 20, 30, 40, 50, 60, 70, 80, 90, 100]));
  const r = out.response_time_ms;
  assert.equal(r.p50, 50);
  assert.equal(r.p90, 90);
  assert.equal(r.p95, 100);
  assert.equal(r.p99, 100);
});

test("nearest-rank percentiles on 1..100", () => {
  const vals = Array.from({ length: 100 }, (_, i) => i + 1);
  const r = runJson([], withTimes(vals)).response_time_ms;
  assert.equal(r.count, 100);
  assert.equal(r.p50, 50);
  assert.equal(r.p90, 90);
  assert.equal(r.p95, 95);
  assert.equal(r.p99, 99);
});

test("unsorted input is sorted before ranking", () => {
  const r = runJson([], withTimes([90, 10, 50, 70, 30])).response_time_ms;
  assert.equal(r.min, 10);
  assert.equal(r.max, 90);
  assert.equal(r.p50, 50);
  assert.equal(r.p90, 90);
});

test("a single value is every percentile", () => {
  const r = runJson([], withTimes([42])).response_time_ms;
  assert.equal(r.count, 1);
  for (const k of ["min", "max", "mean", "p50", "p90", "p95", "p99"]) assert.equal(r[k], 42, k);
});

test("the time can follow the byte count in common format", () => {
  const out = runJson([], lines(line({ rt: 5 }), line({ rt: 15 })));
  assert.equal(out.response_time_ms.count, 2);
  assert.equal(out.response_time_ms.max, 15);
  assert.equal(out.malformed, 0);
});

test("lines without a time are counted as requests but not in the time statistics", () => {
  const out = runJson([], lines(line({ rt: 100 }), line({}), line({ combined: true }), line({ rt: 300, combined: true })));
  assert.equal(out.requests, 4);
  assert.equal(out.response_time_ms.count, 2);
  assert.equal(out.response_time_ms.mean, 200);
});

test("the mean is rounded to two decimals", () => {
  assert.equal(runJson([], withTimes([1, 2, 2])).response_time_ms.mean, 1.67);
  assert.equal(runJson([], withTimes([0.5, 1.25])).response_time_ms.mean, 0.88);
});

test("decimal response times are read as numbers", () => {
  const r = runJson([], withTimes([0.5, 120.75, 3])).response_time_ms;
  assert.equal(r.min, 0.5);
  assert.equal(r.max, 120.75);
  assert.equal(r.p50, 3);
});

test("--percentiles replaces the default list", () => {
  const vals = Array.from({ length: 100 }, (_, i) => i + 1);
  const r = runJson(["--percentiles", "25,99"], withTimes(vals)).response_time_ms;
  assert.equal(r.p25, 25);
  assert.equal(r.p99, 99);
  assert.equal("p50" in r, false);
  assert.equal("p90" in r, false);
  assert.equal("p95" in r, false);
  const hundred = runJson(["--percentiles=100,1"], withTimes(vals)).response_time_ms;
  assert.equal(hundred.p100, 100);
  assert.equal(hundred.p1, 1);
});

test("invalid --percentiles is a usage error (exit 2, stderr only)", () => {
  for (const bad of ["0", "101", "abc", "50,,90", "50.5"]) {
    const r = run(["--json", "--percentiles", bad], withTimes([1, 2, 3]));
    assert.equal(r.code, 2, bad);
    assert.equal(r.stdout, "", bad);
    assert.match(r.stderr, /^logstat: /, bad);
  }
});

test("response_time_ms is null when no line has a time", () => {
  const out = runJson([], lines(line({}), line({ combined: true })));
  assert.equal(out.response_time_ms, null);
  assert.equal(out.requests, 2);
});
