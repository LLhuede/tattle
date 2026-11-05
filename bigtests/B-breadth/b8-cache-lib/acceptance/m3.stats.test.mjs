import { test } from "node:test";
import assert from "node:assert/strict";
import { importApp } from "../../../common/harness.mjs";

const { createCache } = await importApp();

function fakeClock(start = 0) {
  const c = { t: start, now: () => c.t, advance: (ms) => (c.t += ms) };
  return c;
}

test("a new cache has all-zero stats with exactly the five documented properties", () => {
  assert.deepEqual(createCache().stats(), { hits: 0, misses: 0, evictions: 0, expirations: 0, size: 0 });
});

test("get counts hits and misses", () => {
  const c = createCache();
  c.set("a", 1);
  c.get("a");
  c.get("a");
  c.get("b");
  const s = c.stats();
  assert.equal(s.hits, 2);
  assert.equal(s.misses, 1);
});

test("a get of a stored undefined value is a hit", () => {
  const c = createCache();
  c.set("a", undefined);
  c.get("a");
  assert.equal(c.stats().hits, 1);
  assert.equal(c.stats().misses, 0);
});

test("has, peek and set are not counted as hits or misses", () => {
  const c = createCache();
  c.set("a", 1);
  c.has("a");
  c.has("zzz");
  c.peek("a");
  c.peek("zzz");
  const s = c.stats();
  assert.equal(s.hits, 0);
  assert.equal(s.misses, 0);
});

test("LRU evictions are counted", () => {
  const c = createCache({ maxSize: 2 });
  c.set("a", 1).set("b", 2).set("c", 3).set("d", 4);
  const s = c.stats();
  assert.equal(s.evictions, 2);
  assert.equal(s.size, 2);
  assert.equal(s.expirations, 0);
});

test("an expired entry is counted once however often it is observed; a get on it is a miss", () => {
  const clk = fakeClock();
  const c = createCache({ ttl: 10, clock: clk.now });
  c.set("a", 1);
  clk.advance(10);
  c.has("a");
  c.get("a");
  c.keys();
  const s = c.stats();
  assert.equal(s.expirations, 1);
  assert.equal(s.misses, 1);
  assert.equal(s.hits, 0);
  assert.equal(s.size, 0);
});

test("stats() itself removes and counts expired entries", () => {
  const clk = fakeClock();
  const c = createCache({ clock: clk.now });
  c.set("a", 1, { ttl: 5 }).set("b", 2, { ttl: 5 }).set("c", 3);
  clk.advance(5);
  const s = c.stats();
  assert.equal(s.expirations, 2);
  assert.equal(s.size, 1);
});

test("an expired entry cleared by a set is an expiration, not an eviction", () => {
  const clk = fakeClock();
  const c = createCache({ maxSize: 2, clock: clk.now });
  c.set("a", 1, { ttl: 10 }).set("b", 2);
  clk.advance(10);
  c.set("c", 3);
  const s = c.stats();
  assert.equal(s.expirations, 1);
  assert.equal(s.evictions, 0);
  assert.equal(s.size, 2);
});

test("delete, clear and overwriting count as neither evictions nor expirations", () => {
  const c = createCache({ maxSize: 5 });
  c.set("a", 1).set("b", 2).set("a", 3);
  c.delete("b");
  c.clear();
  const s = c.stats();
  assert.equal(s.evictions, 0);
  assert.equal(s.expirations, 0);
  assert.equal(s.size, 0);
});

test("resetStats zeroes the counters but keeps the entries", () => {
  const c = createCache({ maxSize: 1 });
  c.set("a", 1).set("b", 2);
  c.get("b");
  c.get("a");
  c.resetStats();
  assert.deepEqual(c.stats(), { hits: 0, misses: 0, evictions: 0, expirations: 0, size: 1 });
  assert.equal(c.get("b"), 2);
  assert.equal(c.stats().hits, 1);
});

test("stats() returns a snapshot that later activity or edits don't change", () => {
  const c = createCache();
  c.set("a", 1);
  const before = c.stats();
  c.get("a");
  assert.equal(before.hits, 0);
  before.hits = 99;
  assert.equal(c.stats().hits, 1);
});
