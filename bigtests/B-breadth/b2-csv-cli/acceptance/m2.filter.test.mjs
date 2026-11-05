import { test } from "node:test";
import assert from "node:assert/strict";
import { run, file, ok, PEOPLE } from "./_util.mjs";

const names = (r) => r.stdout.trim().split("\n").slice(1).map((l) => l.split(",")[0]);
const filt = (...conds) => ok(run(["filter", ...conds.flatMap((c) => ["--where", c])], PEOPLE));

test("filter = on text keeps the matching rows and all columns", () => {
  const r = filt("city=Paris");
  assert.equal(r.stdout, "name,age,city,amount\nAlice,30,Paris,10.5\nCarol,35,Paris,5\n");
});

test("filter != on text", () => {
  assert.deepEqual(names(filt("city!=Paris")), ["Bob", "Dave"]);
});

test("numeric comparison: 10 > 9, not text order", () => {
  const input = "n\n9\n10\n100\n2\n";
  const r = ok(run(["filter", "--where", "n>9"], input));
  assert.equal(r.stdout, "n\n10\n100\n");
  const le = ok(run(["filter", "--where", "n<=10"], input));
  assert.equal(le.stdout, "n\n9\n10\n2\n");
});

test("numbers equal as numbers: 1.0 = 1 and -3 < 2", () => {
  const input = "n\n1\n1.0\n-3\n2\n";
  assert.equal(ok(run(["filter", "--where", "n=1"], input)).stdout, "n\n1\n1.0\n");
  assert.equal(ok(run(["filter", "--where", "n<2"], input)).stdout, "n\n1\n1.0\n-3\n");
});

test(">= and <= are recognised before > and <", () => {
  assert.deepEqual(names(filt("age>=30")), ["Alice", "Carol"]);
  assert.deepEqual(names(filt("age<=25")), ["Bob", "Dave"]);
  assert.deepEqual(names(filt("age>30")), ["Carol"]);
  assert.deepEqual(names(filt("age<30")), ["Bob", "Dave"]);
});

test("text comparison is case-sensitive, character by character", () => {
  const input = "w\nB\na\nb\nA\n";
  assert.equal(ok(run(["filter", "--where", "w<a"], input)).stdout, "w\nB\nA\n");
  assert.equal(ok(run(["filter", "--where", "w=a"], input)).stdout, "w\na\n");
});

test("~ is a case-sensitive substring test", () => {
  assert.deepEqual(names(filt("name~a")), ["Carol", "Dave"]);
  assert.deepEqual(names(filt("name~A")), ["Alice"]);
});

test("several --where conditions are ANDed", () => {
  assert.deepEqual(names(filt("city=Paris", "age>30")), ["Carol"]);
  assert.deepEqual(names(filt("city=Paris", "age>30", "name=Nobody")), []);
});

test("the value is everything after the first operator, and may be empty", () => {
  const input = "k,v\na,x=y\nb,\nc,z\n";
  assert.equal(ok(run(["filter", "--where", "v=x=y"], input)).stdout, "k,v\na,x=y\n");
  assert.equal(ok(run(["filter", "--where", "v="], input)).stdout, "k,v\nb,\n");
  assert.equal(ok(run(["filter", "--where=v!="], input)).stdout, "k,v\na,x=y\nc,z\n");
});

test("filter with no matches prints only the header (csv) or [] (json)", () => {
  assert.equal(filt("name=Zed").stdout, "name,age,city,amount\n");
  const j = ok(run(["filter", "--where", "name=Zed", "--format", "json"], PEOPLE));
  assert.deepEqual(JSON.parse(j.stdout), []);
});

test("filter errors: missing --where, unknown column are exit 2; file input works", () => {
  assert.equal(run(["filter"], PEOPLE).code, 2);
  const r = run(["filter", "--where", "country=FR"], PEOPLE);
  assert.equal(r.code, 2);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /country/);
  const f = file(PEOPLE);
  assert.deepEqual(names(ok(run(["filter", f, "--where", "name=Bob"]))), ["Bob"]);
});
