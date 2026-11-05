import { test } from "node:test";
import assert from "node:assert/strict";
import { run, ok, PEOPLE } from "./_util.mjs";

const SALES = "region,product,amount\nnorth,pen,10\nsouth,pen,20\nnorth,ink,5.5\nsouth,ink,\neast,pen,4\n";

test("count without group-by is one row named count", () => {
  assert.equal(ok(run(["aggregate", "--fn", "count"], SALES)).stdout, "count\n5\n");
  const j = JSON.parse(ok(run(["aggregate", "--fn", "count", "--format", "json"], SALES)).stdout);
  assert.deepEqual(j, [{ count: 5 }]);
  assert.equal(typeof j[0].count, "number");
  assert.equal(ok(run(["aggregate", "--fn", "count", "--column", "amount"], SALES)).stdout, "count\n5\n");
});

test("sum, min, max and avg name the column <fn>_<column> and skip empty values", () => {
  assert.equal(ok(run(["aggregate", "--fn", "sum", "--column", "amount"], SALES)).stdout, "sum_amount\n39.5\n");
  assert.equal(ok(run(["aggregate", "--fn", "min", "--column", "amount"], SALES)).stdout, "min_amount\n4\n");
  assert.equal(ok(run(["aggregate", "--fn", "max", "--column", "amount"], SALES)).stdout, "max_amount\n20\n");
  assert.equal(ok(run(["aggregate", "--fn", "avg", "--column", "amount"], SALES)).stdout, "avg_amount\n9.875\n");
});

test("avg of 1 and 2 is 1.5 and JSON values are numbers", () => {
  const j = JSON.parse(ok(run(["aggregate", "--fn", "avg", "--column", "v", "--format", "json"], "v\n1\n2\n")).stdout);
  assert.deepEqual(j, [{ avg_v: 1.5 }]);
});

test("group-by gives one row per value in first-appearance order, group column first", () => {
  const r = ok(run(["aggregate", "--fn", "sum", "--column", "amount", "--group-by", "region"], SALES));
  assert.equal(r.stdout, "region,sum_amount\nnorth,15.5\nsouth,20\neast,4\n");
  const c = ok(run(["aggregate", "--fn", "count", "--group-by", "product"], SALES));
  assert.equal(c.stdout, "product,count\npen,3\nink,2\n");
});

test("group-by JSON keeps the group as a string and the result as a number", () => {
  const j = JSON.parse(ok(run(["aggregate", "--fn", "max", "--column", "amount", "--group-by", "product", "--format", "json"], SALES)).stdout);
  assert.deepEqual(j, [{ product: "pen", max_amount: 20 }, { product: "ink", max_amount: 5.5 }]);
});

test("a group with no numeric values gives an empty CSV value and null in JSON", () => {
  const input = "g,v\na,1\nb,\nb,\n";
  assert.equal(ok(run(["aggregate", "--fn", "sum", "--column", "v", "--group-by", "g"], input)).stdout, "g,sum_v\na,1\nb,\n");
  const j = JSON.parse(ok(run(["aggregate", "--fn", "avg", "--column", "v", "--group-by", "g", "--format", "json"], input)).stdout);
  assert.deepEqual(j, [{ g: "a", avg_v: 1 }, { g: "b", avg_v: null }]);
});

test("a non-numeric value in the column is exit 1 and names the column and the value", () => {
  const r = run(["aggregate", "--fn", "sum", "--column", "name"], PEOPLE);
  assert.equal(r.code, 1);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /^error: /m);
  assert.match(r.stderr, /name/);
  assert.match(r.stderr, /Alice/);
});

test("aggregate usage errors are exit 2", () => {
  for (const args of [["aggregate"], ["aggregate", "--fn", "median", "--column", "amount"], ["aggregate", "--fn", "sum"], ["aggregate", "--fn", "sum", "--column", "nope"], ["aggregate", "--fn", "count", "--group-by", "nope"]]) {
    const r = run(args, SALES);
    assert.equal(r.code, 2, args.join(" "));
    assert.equal(r.stdout, "");
    assert.match(r.stderr, /^error: /m);
  }
  assert.match(run(["aggregate", "--fn", "sum", "--column", "nope"], SALES).stderr, /nope/);
});

test("--help prints usage naming every subcommand to stdout and exits 0", () => {
  for (const flag of ["--help", "-h"]) {
    const r = run([flag]);
    assert.equal(r.code, 0);
    for (const s of ["select", "filter", "sort", "aggregate"]) assert.match(r.stdout, new RegExp(s));
  }
});

test("no subcommand: usage on stderr, nothing on stdout, exit 2", () => {
  const r = run([]);
  assert.equal(r.code, 2);
  assert.equal(r.stdout, "");
  assert.ok(r.stderr.length > 0);
});

test("unknown subcommand is exit 2 and names it", () => {
  const r = run(["frobnicate"], PEOPLE);
  assert.equal(r.code, 2);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /^error: /m);
  assert.match(r.stderr, /frobnicate/);
});

test("malformed input is exit 1 with the row number, also for aggregate", () => {
  const r = run(["aggregate", "--fn", "count"], "a,b\n1,2\n3,4\n5\n");
  assert.equal(r.code, 1);
  assert.match(r.stderr, /row 4/);
});
