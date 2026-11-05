import { test } from "node:test";
import assert from "node:assert/strict";
import { importApp } from "../../../common/harness.mjs";

const { createCache } = await importApp();

function fakeClock(start = 0) {
  const c = { t: start, now: () => c.t, advance: (ms) => (c.t += ms) };
  return c;
}

test("an entry with the default ttl is gone at exactly ttl ms", () => {
  const clk = fakeClock(1000);
  const c = createCache({ ttl: 100, clock: clk.now });
  c.set("a", 1);
  clk.advance(99);
  assert.equal(c.get("a"), 1);
  clk.advance(1);
  assert.equal(c.get("a"), undefined);
  assert.equal(c.has("a"), false);
});

test("per-key ttl overrides the default, shorter or longer", () => {
  const clk = fakeClock();
  const c = createCache({ ttl: 100, clock: clk.now });
  c.set("short", 1, { ttl: 10 }).set("long", 2, { ttl: 500 }).set("default", 3);
  clk.advance(10);
  assert.equal(c.has("short"), false);
  assert.equal(c.has("default"), true);
  clk.advance(90);
  assert.equal(c.has("default"), false);
  assert.equal(c.get("long"), 2);
  clk.advance(400);
  assert.equal(c.has("long"), false);
});

test("per-key ttl Infinity never expires even with a default ttl", () => {
  const clk = fakeClock();
  const c = createCache({ ttl: 50, clock: clk.now });
  c.set("forever", 1, { ttl: Infinity }).set("normal", 2);
  clk.advance(1e9);
  assert.equal(c.get("forever"), 1);
  assert.equal(c.has("normal"), false);
});

test("without any ttl entries never expire", () => {
  const clk = fakeClock();
  const c = createCache({ clock: clk.now });
  c.set("a", 1);
  clk.advance(Number.MAX_SAFE_INTEGER / 2);
  assert.equal(c.get("a"), 1);
});

test("overwriting resets the expiry from the time of the new set", () => {
  const clk = fakeClock();
  const c = createCache({ ttl: 100, clock: clk.now });
  c.set("a", 1);
  clk.advance(60);
  c.set("a", 2);
  clk.advance(90);
  assert.equal(c.get("a"), 2);
  clk.advance(10);
  assert.equal(c.get("a"), undefined);
});

test("get does not extend the expiry", () => {
  const clk = fakeClock();
  const c = createCache({ ttl: 100, clock: clk.now });
  c.set("a", 1);
  clk.advance(50);
  assert.equal(c.get("a"), 1);
  clk.advance(50);
  assert.equal(c.get("a"), undefined);
});

test("peek returns undefined for an expired entry", () => {
  const clk = fakeClock();
  const c = createCache({ ttl: 10, clock: clk.now });
  c.set("a", 1);
  assert.equal(c.peek("a"), 1);
  clk.advance(10);
  assert.equal(c.peek("a"), undefined);
});

test("size and keys() leave out expired entries; delete of an expired key is false", () => {
  const clk = fakeClock();
  const c = createCache({ clock: clk.now });
  c.set("a", 1, { ttl: 10 }).set("b", 2).set("c", 3, { ttl: 10 });
  assert.equal(c.size, 3);
  clk.advance(10);
  assert.equal(c.size, 1);
  assert.deepEqual(c.keys(), ["b"]);
  c.set("d", 4, { ttl: 5 });
  clk.advance(5);
  assert.equal(c.delete("d"), false);
});

test("expired entries don't count against maxSize", () => {
  const clk = fakeClock();
  const c = createCache({ maxSize: 2, clock: clk.now });
  c.set("a", 1, { ttl: 10 }).set("b", 2);
  clk.advance(10);
  c.set("c", 3);
  assert.deepEqual(c.keys(), ["b", "c"]);
});

test("ttlRemaining gives the ms left, Infinity for no expiry, undefined when absent", () => {
  const clk = fakeClock(5000);
  const c = createCache({ clock: clk.now });
  c.set("a", 1, { ttl: 100 }).set("b", 2);
  clk.advance(30);
  assert.equal(c.ttlRemaining("a"), 70);
  assert.equal(c.ttlRemaining("b"), Infinity);
  assert.equal(c.ttlRemaining("zzz"), undefined);
  clk.advance(70);
  assert.equal(c.ttlRemaining("a"), undefined);
});

test("invalid ttl throws TypeError, per key too, and a failed set changes nothing", () => {
  for (const bad of [0, -5, NaN, "10"]) {
    assert.throws(() => createCache({ ttl: bad }), TypeError, `ttl ${String(bad)}`);
  }
  const c = createCache();
  c.set("keep", 1);
  assert.throws(() => c.set("x", 2, { ttl: 0 }), TypeError);
  assert.throws(() => c.set("keep", 99, { ttl: -1 }), TypeError);
  assert.equal(c.get("keep"), 1);
  assert.equal(c.has("x"), false);
});

test("the default clock is Date.now (real time)", async () => {
  const c = createCache({ ttl: 50 });
  c.set("a", 1);
  assert.equal(c.get("a"), 1);
  await new Promise((r) => setTimeout(r, 200));
  assert.equal(c.get("a"), undefined);
});
