import { test } from "node:test";
import assert from "node:assert/strict";
import { importApp } from "../../../common/harness.mjs";

const { sessionStats } = await importApp("src/lib/scheduler.js");

test("an empty session is all zeros", () => {
  assert.deepEqual(sessionStats([]), { total: 0, correct: 0, accuracy: 0, points: 0, bestStreak: 0 });
});

test("a mixed session: points, accuracy and best streak", () => {
  // 10, 12, 19, 0, 10
  assert.deepEqual(sessionStats([2, 2, 3, 0, 2]), { total: 5, correct: 4, accuracy: 0.8, points: 51, bestStreak: 3 });
});

test("base points per grade with no streak bonus: 0, 5, 10, 15", () => {
  assert.equal(sessionStats([0]).points, 0);
  assert.equal(sessionStats([1]).points, 5);
  assert.equal(sessionStats([2]).points, 10);
  assert.equal(sessionStats([3]).points, 15);
});

test("the streak bonus grows by 2 per answer and is capped at 10", () => {
  // 8 Goods: bonuses 0,2,4,6,8,10,10,10 = 50, plus 8 * 10
  const s = sessionStats(Array(8).fill(2));
  assert.equal(s.points, 130);
  assert.equal(s.bestStreak, 8);
  assert.equal(s.correct, 8);
  assert.equal(s.accuracy, 1);
});

test("Hard and Again both reset the streak and earn no bonus", () => {
  assert.equal(sessionStats([2, 2, 1, 2, 2]).points, 10 + 12 + 5 + 10 + 12);
  assert.equal(sessionStats([3, 3, 0, 3, 3]).points, 15 + 17 + 0 + 15 + 17);
  assert.equal(sessionStats([2, 2, 1, 2]).bestStreak, 2);
});

test("Hard does not count as correct", () => {
  const s = sessionStats([1, 1, 2, 0]);
  assert.equal(s.correct, 1);
  assert.equal(s.accuracy, 0.25);
});

test("accuracy is rounded to two decimals", () => {
  assert.equal(sessionStats([2, 2, 0]).accuracy, 0.67);
  assert.equal(sessionStats([2, 0, 0]).accuracy, 0.33);
  assert.equal(sessionStats([0, 0]).accuracy, 0);
});

test("best streak is the longest run, even if it is not the last", () => {
  assert.equal(sessionStats([2, 3, 3, 3, 0, 2, 2]).bestStreak, 4);
  assert.equal(sessionStats([0, 1, 0]).bestStreak, 0);
});

test("an invalid grade anywhere throws RangeError; the input array is not changed", () => {
  for (const bad of [[2, 4], [-1], [1.5], ["2"], [2, 2, NaN]]) {
    assert.throws(() => sessionStats(bad), RangeError, JSON.stringify(bad));
  }
  const g = Object.freeze([2, 3, 1]);
  assert.equal(sessionStats(g).total, 3);
});
