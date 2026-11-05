import { test } from "node:test";
import assert from "node:assert/strict";
import { importApp } from "../../../common/harness.mjs";

const { review, newCardState, DAY_MS, LEARN_STEP_MS } = await importApp("src/lib/scheduler.js");

const NOW = 1_700_000_000_000;
const st = (o) => ({ ease: 2.5, interval: 0, reps: 0, lapses: 0, due: 0, ...o });

test("a first Good review: interval 1 day, reps 1, ease unchanged", () => {
  assert.deepEqual(review(newCardState(NOW), 2, NOW), { ease: 2.5, interval: 1, reps: 1, lapses: 0, due: NOW + DAY_MS });
});

test("Good, Good, Good, Good gives intervals 1, 3, 8, 20", () => {
  let s = newCardState(NOW);
  const seen = [];
  for (let i = 0; i < 4; i++) {
    s = review(s, 2, NOW);
    seen.push(s.interval);
  }
  assert.deepEqual(seen, [1, 3, 8, 20]);
  assert.equal(s.reps, 4);
  assert.equal(s.ease, 2.5);
  assert.equal(s.due, NOW + 20 * DAY_MS);
});

test("first Easy review: 4 days and ease 2.65; second Easy: 7 days and ease 2.8", () => {
  const a = review(newCardState(NOW), 3, NOW);
  assert.deepEqual(a, { ease: 2.65, interval: 4, reps: 1, lapses: 0, due: NOW + 4 * DAY_MS });
  const b = review(a, 3, NOW);
  assert.equal(b.interval, 7);
  assert.equal(b.ease, 2.8);
  assert.equal(b.reps, 2);
});

test("Easy on a mature card uses the raised ease and the 1.3 bonus", () => {
  // ease 2.5 -> 2.65; 10 * 2.65 * 1.3 = 34.45 -> 34
  assert.equal(review(st({ interval: 10, reps: 3 }), 3, NOW).interval, 34);
  // ease 3 -> 3.15; 20 * 3.15 * 1.3 = 81.9 -> 82
  assert.deepEqual(review(st({ ease: 3, interval: 20, reps: 2 }), 3, NOW), { ease: 3.15, interval: 82, reps: 3, lapses: 0, due: NOW + 82 * DAY_MS });
});

test("Hard: ease drops 0.15; interval is 1 for a new card and 1.2x (rounded) after", () => {
  assert.deepEqual(review(newCardState(NOW), 1, NOW), { ease: 2.35, interval: 1, reps: 1, lapses: 0, due: NOW + DAY_MS });
  const later = review(st({ interval: 10, reps: 5 }), 1, NOW);
  assert.equal(later.interval, 12);
  assert.equal(later.ease, 2.35);
  assert.equal(later.reps, 6);
  assert.equal(review(st({ interval: 1, reps: 1 }), 1, NOW).interval, 1);
  assert.equal(review(st({ interval: 3, reps: 2 }), 1, NOW).interval, 4);
});

test("Good on a mature card multiplies the interval by the ease", () => {
  assert.equal(review(st({ interval: 10, reps: 5 }), 2, NOW).interval, 25);
  assert.equal(review(st({ ease: 1.3, interval: 10, reps: 3 }), 2, NOW).interval, 13);
  assert.equal(review(st({ interval: 1, reps: 2 }), 2, NOW).interval, 3);
});

test("Again resets reps, adds a lapse, lowers ease and schedules a 10 minute step", () => {
  const r = review(st({ ease: 2.5, interval: 20, reps: 4, lapses: 1 }), 0, NOW);
  assert.deepEqual(r, { ease: 2.3, interval: 0, reps: 0, lapses: 2, due: NOW + LEARN_STEP_MS });
});

test("ease never drops below 1.3", () => {
  assert.equal(review(st({ ease: 1.4, interval: 5, reps: 2 }), 0, NOW).ease, 1.3);
  assert.equal(review(st({ ease: 1.3, interval: 5, reps: 2 }), 1, NOW).ease, 1.3);
  assert.equal(review(st({ ease: 1.35, interval: 5, reps: 2 }), 1, NOW).ease, 1.3);
  let s = newCardState(NOW);
  for (let i = 0; i < 20; i++) s = review(s, 0, NOW);
  assert.equal(s.ease, 1.3);
  assert.equal(s.lapses, 20);
});

test("after a lapse the card starts over: Good gives 1 day, then 3", () => {
  let s = review(st({ interval: 30, reps: 6 }), 0, NOW);
  s = review(s, 2, NOW);
  assert.equal(s.interval, 1);
  assert.equal(s.reps, 1);
  s = review(s, 2, NOW);
  assert.equal(s.interval, 3);
  assert.equal(s.lapses, 1);
});

test("ease is rounded to two decimals after every review", () => {
  let s = newCardState(NOW);
  for (const g of [1, 1, 3, 0, 1, 3, 3]) s = review(s, g, NOW);
  assert.equal(s.ease, Math.round(s.ease * 100) / 100);
  // 2.5 -.15 -.15 +.15 -.2 -.15 +.15 +.15 = 2.3
  assert.equal(s.ease, 2.3);
});

test("review is pure: it doesn't change the input state, and returns a new object with five properties", () => {
  const input = Object.freeze(st({ interval: 10, reps: 5, lapses: 2, due: 123 }));
  const out = review(input, 2, NOW);
  assert.notEqual(out, input);
  assert.deepEqual(Object.keys(out).sort(), ["due", "ease", "interval", "lapses", "reps"]);
  assert.deepEqual(input, { ease: 2.5, interval: 10, reps: 5, lapses: 2, due: 123 });
});

test("an invalid grade throws RangeError", () => {
  for (const bad of [-1, 4, 1.5, NaN, "2", undefined, null]) {
    assert.throws(() => review(newCardState(NOW), bad, NOW), RangeError, String(bad));
  }
});
