import { test } from "node:test";
import assert from "node:assert/strict";
import { load, isMoney, amounts } from "./_api.mjs";

const { money, multiply, allocate, split, add } = await load();
const sumOf = (parts) => parts.reduce((a, p) => a + BigInt(p.amount), 0n);

test("multiply rounds half away from zero", () => {
  isMoney(multiply(money(1, "USD"), 2.5), 3, "USD");
  isMoney(multiply(money(-1, "USD"), 2.5), -3, "USD");
  isMoney(multiply(money(7, "USD"), 0.5), 4, "USD");
  isMoney(multiply(money(-7, "USD"), 0.5), -4, "USD");
  isMoney(multiply(money(3, "USD"), 0.4), 1, "USD");
  isMoney(multiply(money(1000, "USD"), 3), 3000, "USD");
});

test("multiply uses the exact decimal value of the factor", () => {
  isMoney(multiply(money(100, "USD"), 1.005), 101, "USD");
  isMoney(multiply(money(2675, "USD"), 0.01), 27, "USD");
  isMoney(multiply(money(1999, "USD"), 0.075), 150, "USD");
  isMoney(multiply(money(3, "USD"), 0.15), 0, "USD");
  isMoney(multiply(money(10, "USD"), 0.45), 5, "USD");
});

test("multiply never returns -0 and handles zero and negative factors", () => {
  isMoney(multiply(money(1, "USD"), -0.1), 0, "USD");
  isMoney(multiply(money(0, "USD"), 5), 0, "USD");
  isMoney(multiply(money(100, "USD"), 0), 0, "USD");
  isMoney(multiply(money(100, "USD"), -1.5), -150, "USD");
  isMoney(multiply(money(-100, "USD"), -0.015), 2, "USD");
});

test("multiply keeps the currency, does not mutate, and rejects bad factors and overflow", () => {
  const m = money(500, "JPY");
  isMoney(multiply(m, 2), 1000, "JPY");
  isMoney(m, 500, "JPY");
  for (const bad of [NaN, Infinity, "2", undefined, null]) assert.throws(() => multiply(m, bad), Error, String(bad));
  assert.throws(() => multiply(money(Number.MAX_SAFE_INTEGER, "USD"), 2), Error);
});

test("allocate: the classic 100 split three ways gives the extra cent to the first part", () => {
  const parts = allocate(money(100, "USD"), [1, 1, 1]);
  assert.deepEqual(amounts(parts), [34, 33, 33]);
  for (const p of parts) assert.equal(p.currency, "USD");
});

test("allocate by uneven ratios hands leftovers out in order", () => {
  assert.deepEqual(amounts(allocate(money(5, "USD"), [3, 7])), [2, 3]);
  assert.deepEqual(amounts(allocate(money(10, "USD"), [1, 2, 3])), [2, 3, 5]);
  assert.deepEqual(amounts(allocate(money(11, "USD"), [1, 1, 1, 1])), [3, 3, 3, 2]);
  assert.deepEqual(amounts(allocate(money(1, "USD"), [1, 1, 1])), [1, 0, 0]);
  assert.deepEqual(amounts(allocate(money(7, "EUR"), [50, 30, 20])), [4, 2, 1]);
});

test("allocate: parts with ratio 0 always get 0, even when there are leftovers", () => {
  assert.deepEqual(amounts(allocate(money(1, "USD"), [0, 1, 1])), [0, 1, 0]);
  assert.deepEqual(amounts(allocate(money(5, "USD"), [0, 1, 0, 1])), [0, 3, 0, 2]);
  assert.deepEqual(amounts(allocate(money(100, "USD"), [0, 0, 5])), [0, 0, 100]);
});

test("allocate on negative and zero amounts", () => {
  assert.deepEqual(amounts(allocate(money(-100, "USD"), [1, 1, 1])), [-34, -33, -33]);
  const zero = allocate(money(0, "USD"), [1, 2]);
  isMoney(zero[0], 0, "USD");
  isMoney(zero[1], 0, "USD");
  const neg = allocate(money(-1, "USD"), [0, 1, 1]);
  isMoney(neg[0], 0, "USD");
  isMoney(neg[1], -1, "USD");
  isMoney(neg[2], 0, "USD");
});

test("allocate loses nothing, even for amounts where amount * ratio exceeds 2^53", () => {
  const big = money(Number.MAX_SAFE_INTEGER, "USD");
  const parts = allocate(big, [1, 1, 1]);
  assert.deepEqual(amounts(parts), [3002399751580331, 3002399751580330, 3002399751580330]);
  assert.equal(sumOf(parts), BigInt(Number.MAX_SAFE_INTEGER));
  const uneven = allocate(big, [123456789, 987654321, 5, 1000]);
  assert.equal(sumOf(uneven), BigInt(Number.MAX_SAFE_INTEGER));
  const exp = (r) => (BigInt(Number.MAX_SAFE_INTEGER) * BigInt(r)) / (123456789n + 987654321n + 5n + 1000n);
  uneven.forEach((p, i) => {
    const floor = exp([123456789, 987654321, 5, 1000][i]);
    assert.ok(BigInt(p.amount) === floor || BigInt(p.amount) === floor + 1n);
  });
});

test("allocate always sums back to the original for many ratio sets", () => {
  const sets = [[1], [1, 1], [3, 1], [1, 1, 1, 1, 1, 1, 1], [70, 20, 10], [0, 7, 0, 3], [2, 3, 5, 7, 11, 13]];
  for (const amount of [0, 1, 2, 99, 100, 101, 12345, -1, -99, -12345]) {
    for (const ratios of sets) {
      const parts = allocate(money(amount, "USD"), ratios);
      assert.equal(parts.length, ratios.length);
      assert.equal(sumOf(parts), BigInt(amount), `${amount} / ${ratios}`);
    }
  }
});

test("allocate rejects bad ratios", () => {
  const m = money(100, "USD");
  for (const bad of [[], [0, 0], [-1, 2], [1.5, 1], [1, NaN], "1,1", undefined, [1, "2"], [Number.MAX_SAFE_INTEGER + 2, 1]]) {
    assert.throws(() => allocate(m, bad), Error, JSON.stringify(bad));
  }
});

test("split is an equal allocation", () => {
  assert.deepEqual(amounts(split(money(100, "USD"), 3)), [34, 33, 33]);
  assert.deepEqual(amounts(split(money(100, "USD"), 4)), [25, 25, 25, 25]);
  assert.deepEqual(amounts(split(money(2, "USD"), 5)), [1, 1, 0, 0, 0]);
  assert.deepEqual(amounts(split(money(-10, "EUR"), 3)), [-4, -3, -3]);
  assert.equal(split(money(10, "JPY"), 1)[0].amount, 10);
  for (const bad of [0, -1, 1.5, NaN, "3", undefined]) assert.throws(() => split(money(100, "USD"), bad), Error, String(bad));
});
