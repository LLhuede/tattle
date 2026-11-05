import { test } from "node:test";
import assert from "node:assert/strict";
import { run, file, ok, PEOPLE } from "./_util.mjs";

test("select reads stdin and outputs the chosen columns as CSV", () => {
  const r = ok(run(["select", "--columns", "name,city"], PEOPLE));
  assert.equal(r.stdout, "name,city\nAlice,Paris\nBob,London\nCarol,Paris\nDave,Rome\n");
  assert.equal(r.stderr, "");
});

test("select reads a file argument, before or after the options, and - means stdin", () => {
  const f = file(PEOPLE);
  const a = ok(run(["select", "--columns", "age", f]));
  const b = ok(run(["select", f, "--columns", "age"]));
  assert.equal(a.stdout, "age\n30\n25\n35\n25\n");
  assert.equal(b.stdout, a.stdout);
  const c = ok(run(["select", "--columns", "age", "-"], PEOPLE));
  assert.equal(c.stdout, a.stdout);
  const e = ok(run(["select", "--columns=city,name", "--format=csv"], PEOPLE));
  assert.equal(e.stdout.split("\n")[0], "city,name");
});

test("select keeps the order given and allows a column twice", () => {
  const r = ok(run(["select", "--columns", "city,name,city"], PEOPLE));
  assert.equal(r.stdout.split("\n")[0], "city,name,city");
  assert.equal(r.stdout.split("\n")[1], "Paris,Alice,Paris");
});

test("--format json prints an array of objects with string values in column order", () => {
  const r = ok(run(["select", "--columns", "age,name", "--format", "json"], PEOPLE));
  assert.ok(r.stdout.endsWith("\n"));
  const rows = JSON.parse(r.stdout);
  assert.equal(rows.length, 4);
  assert.deepEqual(rows[0], { age: "30", name: "Alice" });
  assert.deepEqual(Object.keys(rows[0]), ["age", "name"]);
  assert.equal(typeof rows[1].age, "string");
  const e = JSON.parse(ok(run(["select", "--columns", "name,amount", "--format", "json"], PEOPLE)).stdout);
  assert.deepEqual(e[3], { name: "Dave", amount: "" });
});

test("a CSV with only a header prints the header (csv) or [] (json)", () => {
  const csv = ok(run(["select", "--columns", "a"], "a,b\n"));
  assert.equal(csv.stdout, "a\n");
  const json = ok(run(["select", "--columns", "a", "--format", "json"], "a,b\n"));
  assert.deepEqual(JSON.parse(json.stdout), []);
});

test("quoted fields: commas, doubled quotes and line breaks are data, and output is re-quoted", () => {
  const input = 'id,text\n1,"hello, world"\n2,"she said ""hi"""\n3,"line one\nline two"\n4,plain\n';
  const r = ok(run(["select", "--columns", "id,text"], input));
  assert.equal(r.stdout, input);
  const j = JSON.parse(ok(run(["select", "--columns", "text", "--format", "json"], input)).stdout);
  assert.deepEqual(j.map((x) => x.text), ["hello, world", 'she said "hi"', "line one\nline two", "plain"]);
});

test("CRLF line endings, a missing final newline and blank lines are handled", () => {
  const r = ok(run(["select", "--columns", "b,a"], "a,b\r\n1,2\r\n\r\n3,4"));
  assert.equal(r.stdout, "b,a\n2,1\n4,3\n");
});

test("unknown column: exit 2, nothing on stdout, an error line naming the column", () => {
  const r = run(["select", "--columns", "name,nope"], PEOPLE);
  assert.equal(r.code, 2);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /^error: /m);
  assert.match(r.stderr, /nope/);
});

test("select without --columns is a usage error (exit 2)", () => {
  const r = run(["select"], PEOPLE);
  assert.equal(r.code, 2);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /^error: /m);
});

test("a file that cannot be read: exit 1 and the message includes the file name", () => {
  const r = run(["select", "--columns", "a", "/definitely/not/here/missing-file.csv"]);
  assert.equal(r.code, 1);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /^error: /m);
  assert.match(r.stderr, /missing-file\.csv/);
});

test("malformed CSV: unterminated quote and wrong field count are exit 1 (with the row number)", () => {
  const a = run(["select", "--columns", "a"], 'a,b\n1,"oops\n');
  assert.equal(a.code, 1);
  assert.equal(a.stdout, "");
  assert.match(a.stderr, /^error: /m);
  const b = run(["select", "--columns", "a"], "a,b\n1,2\n3\n5,6\n");
  assert.equal(b.code, 1);
  assert.equal(b.stdout, "");
  assert.match(b.stderr, /row 3/);
  const c = run(["select", "--columns", "a"], "");
  assert.equal(c.code, 1);
});

test("bad --format value or unknown option is exit 2", () => {
  assert.equal(run(["select", "--columns", "name", "--format", "xml"], PEOPLE).code, 2);
  assert.equal(run(["select", "--columns", "name", "--bogus", "1"], PEOPLE).code, 2);
});
