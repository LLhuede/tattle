import { test } from "node:test";
import assert from "node:assert/strict";
import { run, ok, PEOPLE } from "./_util.mjs";

const names = (r) => r.stdout.trim().split("\n").slice(1).map((l) => l.split(",")[0]);
const filt = (...conds) => ok(run(["filter", ...conds.flatMap((c) => ["--where", c])], PEOPLE));

test("sort numeric ascending and descending", () => {
  const input = "id,n\na,9\nb,10\nc,100\nd,-5\n";
  assert.equal(ok(run(["sort", "--by", "n"], input)).stdout, "id,n\nd,-5\na,9\nb,10\nc,100\n");
  assert.equal(ok(run(["sort", "--by", "n", "--desc"], input)).stdout, "id,n\nc,100\nb,10\na,9\nd,-5\n");
});

test("sort by text uses plain character order (uppercase first)", () => {
  const input = "w\nbanana\nApple\napple\nCherry\n";
  assert.equal(ok(run(["sort", "--by", "w"], input)).stdout, "w\nApple\nCherry\napple\nbanana\n");
  assert.equal(ok(run(["sort", "--by", "w", "--desc"], input)).stdout, "w\nbanana\napple\nCherry\nApple\n");
});

test("a column with one non-number sorts as text", () => {
  const input = "v\n9\n10\nx\n";
  assert.equal(ok(run(["sort", "--by", "v"], input)).stdout, "v\n10\n9\nx\n");
});

test("sort is stable, also with --desc", () => {
  const asc = ok(run(["sort", "--by", "age"], PEOPLE));
  assert.deepEqual(names(asc), ["Bob", "Dave", "Alice", "Carol"]);
  const desc = ok(run(["sort", "--by", "age", "--desc"], PEOPLE));
  assert.deepEqual(names(desc), ["Carol", "Alice", "Bob", "Dave"]);
});

test("empty cells sort last in both directions", () => {
  const asc = ok(run(["sort", "--by", "amount"], PEOPLE));
  assert.deepEqual(names(asc), ["Carol", "Alice", "Bob", "Dave"]);
  const desc = ok(run(["sort", "--by", "amount", "--desc"], PEOPLE));
  assert.deepEqual(names(desc), ["Bob", "Alice", "Carol", "Dave"]);
});

test("sort supports json output and its errors", () => {
  const j = JSON.parse(ok(run(["sort", "--by", "name", "--desc", "--format", "json"], PEOPLE)).stdout);
  assert.deepEqual(j.map((r) => r.name), ["Dave", "Carol", "Bob", "Alice"]);
  assert.equal(run(["sort"], PEOPLE).code, 2);
  const bad = run(["sort", "--by", "zip"], PEOPLE);
  assert.equal(bad.code, 2);
  assert.match(bad.stderr, /zip/);
});

test("subcommands chain through pipes", () => {
  const a = ok(run(["filter", "--where", "city=Paris"], PEOPLE));
  const b = ok(run(["sort", "--by", "age", "--desc"], a.stdout));
  const c = ok(run(["select", "--columns", "name"], b.stdout));
  assert.equal(c.stdout, "name\nCarol\nAlice\n");
});
