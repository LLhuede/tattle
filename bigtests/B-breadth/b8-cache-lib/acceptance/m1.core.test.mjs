import { test } from "node:test";
import assert from "node:assert/strict";
import { importApp } from "../../../common/harness.mjs";

const lib = await importApp();
const { createCache } = lib;

test("exports createCache and memoize as functions", () => {
  assert.equal(typeof lib.createCache, "function");
  assert.equal(typeof lib.memoize, "function");
});

test("set then get returns the value; missing keys give undefined", () => {
  const c = createCache();
  c.set("a", 1);
  assert.equal(c.get("a"), 1);
  assert.equal(c.get("nope"), undefined);
});

test("set returns the cache so calls can be chained", () => {
  const c = createCache();
  const r = c.set("a", 1).set("b", 2);
  assert.equal(r, c);
  assert.equal(c.size, 2);
});

test("keys are compared like Map keys (1 and '1' differ, objects by identity)", () => {
  const c = createCache();
  const o = {};
  c.set(1, "number").set("1", "string").set(o, "object");
  assert.equal(c.get(1), "number");
  assert.equal(c.get("1"), "string");
  assert.equal(c.get(o), "object");
  assert.equal(c.get({}), undefined);
  assert.equal(c.size, 3);
});

test("overwriting a key replaces the value and keeps size", () => {
  const c = createCache();
  c.set("a", 1).set("a", 2);
  assert.equal(c.get("a"), 2);
  assert.equal(c.size, 1);
});

test("delete returns true for a present key and false otherwise; clear empties", () => {
  const c = createCache();
  c.set("a", 1).set("b", 2);
  assert.equal(c.delete("a"), true);
  assert.equal(c.delete("a"), false);
  assert.equal(c.has("a"), false);
  c.clear();
  assert.equal(c.size, 0);
  assert.deepEqual(c.keys(), []);
});

test("maxSize evicts the least recently used entry", () => {
  const c = createCache({ maxSize: 3 });
  c.set("a", 1).set("b", 2).set("c", 3).set("d", 4);
  assert.equal(c.has("a"), false);
  assert.deepEqual(c.keys(), ["b", "c", "d"]);
  assert.equal(c.size, 3);
});

test("a get makes the entry most recently used", () => {
  const c = createCache({ maxSize: 3 });
  c.set("a", 1).set("b", 2).set("c", 3);
  assert.equal(c.get("a"), 1);
  c.set("d", 4);
  assert.deepEqual(c.keys(), ["c", "a", "d"]);
  assert.equal(c.has("b"), false);
});

test("overwriting an existing key makes it most recently used and evicts nothing", () => {
  const c = createCache({ maxSize: 2 });
  c.set("a", 1).set("b", 2);
  c.set("a", 10);
  assert.deepEqual(c.keys(), ["b", "a"]);
  assert.equal(c.size, 2);
  assert.equal(c.get("b"), 2);
});

test("has and peek change no recency", () => {
  const c = createCache({ maxSize: 3 });
  c.set("a", 1).set("b", 2).set("c", 3);
  assert.equal(c.has("a"), true);
  assert.equal(c.peek("a"), 1);
  c.set("d", 4);
  assert.equal(c.has("a"), false);
  assert.deepEqual(c.keys(), ["b", "c", "d"]);
});

test("maxSize 1 keeps only the newest entry; without maxSize the cache is unbounded", () => {
  const one = createCache({ maxSize: 1 });
  one.set("a", 1).set("b", 2);
  assert.deepEqual(one.keys(), ["b"]);
  const many = createCache();
  for (let i = 0; i < 1000; i++) many.set(i, i);
  assert.equal(many.size, 1000);
});

test("undefined is a storable value; invalid maxSize throws TypeError", () => {
  const c = createCache();
  c.set("k", undefined);
  assert.equal(c.has("k"), true);
  assert.equal(c.size, 1);
  assert.equal(c.get("k"), undefined);
  for (const bad of [0, -1, 1.5, "3", NaN]) {
    assert.throws(() => createCache({ maxSize: bad }), TypeError, `maxSize ${String(bad)}`);
  }
});
