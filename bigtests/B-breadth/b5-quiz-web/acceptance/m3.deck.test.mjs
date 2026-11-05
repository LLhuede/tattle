import { test } from "node:test";
import assert from "node:assert/strict";
import { importApp } from "../../../common/harness.mjs";

const { dueCards, summarizeDeck, shuffle } = await importApp("src/lib/scheduler.js");

const NOW = 1_700_000_000_000;
const card = (id, o = {}) => ({ id, front: `f${id}`, back: `b${id}`, state: { ease: 2.5, interval: 0, reps: 0, lapses: 0, due: NOW, ...o } });

test("dueCards keeps cards due now or earlier and drops future ones", () => {
  const cards = [card("a", { due: NOW - 1 }), card("b", { due: NOW }), card("c", { due: NOW + 1 })];
  assert.deepEqual(dueCards(cards, NOW).map((c) => c.id), ["a", "b"]);
  assert.deepEqual(dueCards(cards, NOW - 2), []);
  assert.deepEqual(dueCards([], NOW), []);
});

test("dueCards orders by due time, then by id", () => {
  const cards = [card("z", { due: 5 }), card("b", { due: 10 }), card("a", { due: 10 }), card("m", { due: 1 })];
  assert.deepEqual(dueCards(cards, NOW).map((c) => c.id), ["m", "z", "a", "b"]);
});

test("dueCards compares ids as strings", () => {
  const cards = [card("10"), card("9"), card("2")];
  assert.deepEqual(dueCards(cards, NOW).map((c) => c.id), ["10", "2", "9"]);
});

test("dueCards returns the same card objects in a new array and leaves the input alone", () => {
  const cards = [card("b", { due: 2 }), card("a", { due: 1 })];
  const out = dueCards(cards, NOW);
  assert.notEqual(out, cards);
  assert.equal(out[0], cards[1]);
  assert.equal(out[1], cards[0]);
  assert.deepEqual(cards.map((c) => c.id), ["b", "a"]);
});

test("summarizeDeck counts total, due, new and mature", () => {
  const cards = [
    card("1"), // new, due
    card("2", { reps: 3, interval: 10, due: NOW + 1 }), // learning, not due
    card("3", { reps: 6, interval: 21, due: NOW - 5 }), // mature, due
    card("4", { reps: 0, lapses: 1, interval: 0, due: NOW + 600000 }), // lapsed, not new
    card("5", { reps: 9, interval: 20, due: NOW + 1000 }), // 20 is not mature
  ];
  assert.deepEqual(summarizeDeck(cards, NOW), { total: 5, due: 2, new: 1, mature: 1 });
});

test("summarizeDeck of an empty deck is all zeros", () => {
  assert.deepEqual(summarizeDeck([], NOW), { total: 0, due: 0, new: 0, mature: 0 });
});

test("shuffle follows Fisher-Yates from the end with the given rng", () => {
  const seq = [0.99, 0, 0.5];
  let k = 0;
  const out = shuffle([1, 2, 3, 4], () => seq[k++]);
  assert.deepEqual(out, [3, 2, 1, 4]);
  assert.equal(k, 3, "rng called length - 1 times");
  assert.deepEqual(shuffle([1, 2, 3, 4], () => 0), [2, 3, 4, 1]);
  assert.deepEqual(shuffle([1, 2, 3, 4], () => 0.999999), [1, 2, 3, 4]);
});

test("shuffle returns a new array and doesn't touch the input", () => {
  const input = ["a", "b", "c", "d", "e"];
  const out = shuffle(input, () => 0);
  assert.notEqual(out, input);
  assert.deepEqual(input, ["a", "b", "c", "d", "e"]);
  assert.deepEqual([...out].sort(), ["a", "b", "c", "d", "e"]);
});

test("shuffle with 0 or 1 items never calls rng; the default rng keeps all items", () => {
  let calls = 0;
  const rng = () => (calls++, 0.5);
  assert.deepEqual(shuffle([], rng), []);
  assert.deepEqual(shuffle(["x"], rng), ["x"]);
  assert.equal(calls, 0);
  const many = Array.from({ length: 50 }, (_, i) => i);
  const out = shuffle(many);
  assert.equal(out.length, 50);
  assert.deepEqual([...out].sort((a, b) => a - b), many);
});
