import assert from "node:assert/strict";
import { importApp } from "../../../common/harness.mjs";

export async function load() {
  const m = await importApp();
  const d = m.default && typeof m.default === "object" ? m.default : {};
  return { ...d, ...m };
}

/** Assert a money value: exactly { amount, currency }, amount never -0. */
export function isMoney(m, amount, currency) {
  assert.ok(m && typeof m === "object", "expected a money object");
  assert.deepEqual(Object.keys(m).sort(), ["amount", "currency"]);
  assert.ok(Object.is(m.amount, amount), `amount ${Object.is(m.amount, -0) ? "-0" : m.amount} !== ${amount}`);
  assert.equal(m.currency, currency);
}
export const amounts = (list) => list.map((m) => m.amount);
