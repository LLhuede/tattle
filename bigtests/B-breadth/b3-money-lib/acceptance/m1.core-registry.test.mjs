import { test } from "node:test";
import assert from "node:assert/strict";
import { load, isMoney } from "./_api.mjs";

const lib = await load();
const { money, add, subtract, negate, compare, equals, getCurrency, listCurrencies, registerCurrency } = lib;

test("money builds a plain { amount, currency } and never keeps -0", () => {
  isMoney(money(1234, "USD"), 1234, "USD");
  isMoney(money(-5, "JPY"), -5, "JPY");
  isMoney(money(-0, "USD"), 0, "USD");
  isMoney(money(0, "EUR"), 0, "EUR");
});

test("money rejects non-integer, unsafe and non-number amounts", () => {
  for (const bad of [1.5, NaN, Infinity, "100", null, undefined, 2 ** 53, -(2 ** 53)]) {
    assert.throws(() => money(bad, "USD"), Error, String(bad));
  }
  isMoney(money(Number.MAX_SAFE_INTEGER, "USD"), Number.MAX_SAFE_INTEGER, "USD");
});

test("money rejects unregistered currencies, including lower-case codes", () => {
  for (const bad of ["XXX", "usd", "", undefined, 5]) assert.throws(() => money(100, bad), Error, String(bad));
});

test("add and subtract work in minor units without mutating their inputs", () => {
  const a = money(1050, "USD");
  const b = money(295, "USD");
  isMoney(add(a, b), 1345, "USD");
  isMoney(subtract(a, b), 755, "USD");
  isMoney(subtract(b, a), -755, "USD");
  isMoney(a, 1050, "USD");
  isMoney(b, 295, "USD");
  isMoney(add(money(10, "USD"), money(-10, "USD")), 0, "USD");
});

test("add and subtract throw on a currency mismatch", () => {
  assert.throws(() => add(money(1, "USD"), money(1, "EUR")), Error);
  assert.throws(() => subtract(money(1, "USD"), money(1, "EUR")), Error);
});

test("add and subtract throw when the result is not a safe integer", () => {
  assert.throws(() => add(money(Number.MAX_SAFE_INTEGER, "USD"), money(1, "USD")), Error);
  assert.throws(() => subtract(money(-Number.MAX_SAFE_INTEGER, "USD"), money(1, "USD")), Error);
  isMoney(add(money(Number.MAX_SAFE_INTEGER - 1, "USD"), money(1, "USD")), Number.MAX_SAFE_INTEGER, "USD");
});

test("negate flips the sign and never produces -0", () => {
  isMoney(negate(money(250, "GBP")), -250, "GBP");
  isMoney(negate(money(-250, "GBP")), 250, "GBP");
  isMoney(negate(money(0, "USD")), 0, "USD");
});

test("compare returns -1, 0 or 1 and throws across currencies", () => {
  assert.equal(compare(money(1, "USD"), money(2, "USD")), -1);
  assert.equal(compare(money(2, "USD"), money(2, "USD")), 0);
  assert.equal(compare(money(3, "USD"), money(2, "USD")), 1);
  assert.equal(compare(money(-3, "USD"), money(2, "USD")), -1);
  assert.throws(() => compare(money(1, "USD"), money(1, "EUR")), Error);
});

test("equals compares amount and currency, and does not throw across currencies", () => {
  assert.equal(equals(money(5, "USD"), money(5, "USD")), true);
  assert.equal(equals(money(5, "USD"), money(6, "USD")), false);
  assert.equal(equals(money(5, "USD"), money(5, "EUR")), false);
});

test("the registry starts with the six built-in currencies", () => {
  const expected = {
    USD: ["$", 2, "US Dollar"],
    EUR: ["€", 2, "Euro"],
    GBP: ["£", 2, "Pound Sterling"],
    JPY: ["¥", 0, "Yen"],
    CHF: ["CHF", 2, "Swiss Franc"],
    KWD: ["KD", 3, "Kuwaiti Dinar"],
  };
  for (const [code, [symbol, exponent, name]] of Object.entries(expected)) {
    const c = getCurrency(code);
    assert.ok(c, code);
    assert.equal(c.code, code);
    assert.equal(c.symbol, symbol);
    assert.equal(c.exponent, exponent);
    assert.equal(c.name, name);
  }
  assert.equal(getCurrency("usd"), undefined);
  assert.equal(getCurrency("ZZZ"), undefined);
});

test("listCurrencies is sorted by code", () => {
  const codes = listCurrencies().map((c) => c.code);
  assert.deepEqual(codes, ["CHF", "EUR", "GBP", "JPY", "KWD", "USD"]);
});

test("registerCurrency adds a currency, defaults the name, and rejects bad input and duplicates", () => {
  const c = registerCurrency({ code: "SEK", symbol: "kr", exponent: 2 });
  assert.equal(c.code, "SEK");
  assert.equal(c.name, "SEK");
  assert.equal(getCurrency("SEK").symbol, "kr");
  isMoney(money(100, "SEK"), 100, "SEK");
  assert.ok(listCurrencies().map((x) => x.code).includes("SEK"));
  assert.deepEqual(listCurrencies().map((x) => x.code), [...listCurrencies().map((x) => x.code)].sort());
  assert.equal(registerCurrency({ code: "BTC", symbol: "₿", exponent: 6, name: "Bitcoin" }).name, "Bitcoin");
  assert.throws(() => registerCurrency({ code: "SEK", symbol: "kr", exponent: 2 }), Error);
  assert.throws(() => registerCurrency({ code: "USD", symbol: "$", exponent: 2 }), Error);
  for (const bad of [
    { code: "ab", symbol: "x", exponent: 2 },
    { code: "ABCD", symbol: "x", exponent: 2 },
    { code: "abc", symbol: "x", exponent: 2 },
    { code: "XYZ", symbol: "", exponent: 2 },
    { code: "XYZ", symbol: "x", exponent: -1 },
    { code: "XYZ", symbol: "x", exponent: 7 },
    { code: "XYZ", symbol: "x", exponent: 1.5 },
  ]) {
    assert.throws(() => registerCurrency(bad), Error, JSON.stringify(bad));
  }
  assert.equal(getCurrency("XYZ"), undefined);
});
