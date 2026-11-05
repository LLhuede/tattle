import { test } from "node:test";
import assert from "node:assert/strict";
import { importApp } from "../../../common/harness.mjs";

const { memoize } = await importApp();

function fakeClock(start = 0) {
  const c = { t: start, now: () => c.t, advance: (ms) => (c.t += ms) };
  return c;
}

function counted(impl) {
  const f = (...args) => {
    f.calls++;
    return impl(...args);
  };
  f.calls = 0;
  return f;
}

test("fn runs once for repeated calls with the same arguments", () => {
  const f = counted((a, b) => a + b);
  const m = memoize(f);
  assert.equal(m(1, 2), 3);
  assert.equal(m(1, 2), 3);
  assert.equal(f.calls, 1);
});

test("different arguments are cached separately (including 1 vs '1')", () => {
  const f = counted((x) => typeof x);
  const m = memoize(f);
  assert.equal(m(1), "number");
  assert.equal(m("1"), "string");
  assert.equal(m(1), "number");
  assert.equal(f.calls, 2);
  m(1, 2);
  assert.equal(f.calls, 3);
});

test("a custom key function chooses what counts as the same call", () => {
  const f = counted((a, b) => a * 10 + b);
  const m = memoize(f, { key: (a) => a });
  assert.equal(m(1, 1), 11);
  assert.equal(m(1, 2), 11);
  assert.equal(m(2, 2), 22);
  assert.equal(f.calls, 2);
});

test("ttl with an injected clock makes results expire", () => {
  const clk = fakeClock();
  const f = counted((x) => x * 2);
  const m = memoize(f, { ttl: 100, clock: clk.now });
  m(4);
  clk.advance(99);
  m(4);
  assert.equal(f.calls, 1);
  clk.advance(1);
  m(4);
  assert.equal(f.calls, 2);
});

test("maxSize evicts the least recently used result", () => {
  const f = counted((x) => x);
  const m = memoize(f, { maxSize: 2 });
  m(1);
  m(2);
  m(3);
  assert.equal(f.calls, 3);
  m(3);
  m(2);
  assert.equal(f.calls, 3);
  m(1);
  assert.equal(f.calls, 4);
});

test("undefined results are cached", () => {
  const f = counted(() => undefined);
  const m = memoize(f);
  m("x");
  m("x");
  assert.equal(f.calls, 1);
});

test("a throwing fn propagates the error and is not cached", () => {
  let n = 0;
  const m = memoize(() => {
    n++;
    if (n === 1) throw new Error("boom");
    return "ok";
  });
  assert.throws(() => m(1), /boom/);
  assert.equal(m(1), "ok");
  assert.equal(m(1), "ok");
  assert.equal(n, 2);
});

test("a promise result is cached so concurrent callers share one call", async () => {
  const f = counted(async (x) => {
    await new Promise((r) => setTimeout(r, 20));
    return x + 1;
  });
  const m = memoize(f);
  const [a, b] = await Promise.all([m(1), m(1)]);
  assert.equal(a, 2);
  assert.equal(b, 2);
  assert.equal(await m(1), 2);
  assert.equal(f.calls, 1);
});

test("a rejected promise is removed so the next call runs fn again", async () => {
  let n = 0;
  const m = memoize(async () => {
    n++;
    if (n === 1) throw new Error("first fails");
    return "fine";
  });
  await assert.rejects(m("k"), /first fails/);
  assert.equal(await m("k"), "fine");
  assert.equal(await m("k"), "fine");
  assert.equal(n, 2);
});

test("memoized.cache.stats() counts each call as one hit or one miss", () => {
  const m = memoize((x) => x);
  m(1);
  m(1);
  m(1);
  m(2);
  const s = m.cache.stats();
  assert.equal(s.hits, 2);
  assert.equal(s.misses, 2);
  assert.equal(m.cache.size, 2);
});

test("memoized.clear() drops every cached result", () => {
  const f = counted((x) => x);
  const m = memoize(f);
  m(1);
  m(2);
  m.clear();
  assert.equal(m.cache.size, 0);
  m(1);
  assert.equal(f.calls, 3);
});

test("memoize rejects a non-function; this is passed through", () => {
  assert.throws(() => memoize("nope"), TypeError);
  const obj = {
    k: 5,
    f: memoize(function (x) {
      return this.k + x;
    }),
  };
  assert.equal(obj.f(1), 6);
});
