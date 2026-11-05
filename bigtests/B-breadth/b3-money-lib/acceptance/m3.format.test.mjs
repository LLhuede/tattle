import { test } from "node:test";
import assert from "node:assert/strict";
import { load } from "./_api.mjs";

const { money, format } = await load();

test("USD formats with symbol, grouping and two decimals", () => {
  assert.equal(format(money(123456, "USD")), "$1,234.56");
  assert.equal(format(money(5, "USD")), "$0.05");
  assert.equal(format(money(100, "USD")), "$1.00");
  assert.equal(format(money(99, "USD")), "$0.99");
  assert.equal(format(money(100000000, "USD")), "$1,000,000.00");
  assert.equal(format(money(100000, "USD")), "$1,000.00");
  assert.equal(format(money(99999, "USD")), "$999.99");
});

test("zero has no sign and shows the decimals", () => {
  assert.equal(format(money(0, "USD")), "$0.00");
  assert.equal(format(money(0, "JPY")), "¥0");
  assert.equal(format(money(0, "KWD")), "KD0.000");
});

test("negative amounts put - before the symbol", () => {
  assert.equal(format(money(-123456, "USD")), "-$1,234.56");
  assert.equal(format(money(-5, "USD")), "-$0.05");
  assert.equal(format(money(-100, "EUR")), "-€1.00");
});

test("exponent 0 has no decimal point and exponent 3 has three decimals", () => {
  assert.equal(format(money(500, "JPY")), "¥500");
  assert.equal(format(money(1234567, "JPY")), "¥1,234,567");
  assert.equal(format(money(-1000, "JPY")), "-¥1,000");
  assert.equal(format(money(1234, "KWD")), "KD1.234");
  assert.equal(format(money(5, "KWD")), "KD0.005");
  assert.equal(format(money(1234567, "KWD")), "KD1,234.567");
});

test("each built-in symbol is written directly in front of the number", () => {
  assert.equal(format(money(150, "EUR")), "€1.50");
  assert.equal(format(money(150, "GBP")), "£1.50");
  assert.equal(format(money(123450, "CHF")), "CHF1,234.50");
});

test('style "code" puts the code after the number with a space', () => {
  assert.equal(format(money(123456, "USD"), { style: "code" }), "1,234.56 USD");
  assert.equal(format(money(-500, "EUR"), { style: "code" }), "-5.00 EUR");
  assert.equal(format(money(500, "JPY"), { style: "code" }), "500 JPY");
  assert.equal(format(money(0, "GBP"), { style: "code" }), "0.00 GBP");
});

test('style "none" gives just the number', () => {
  assert.equal(format(money(-123456, "USD"), { style: "none" }), "-1,234.56");
  assert.equal(format(money(1234, "KWD"), { style: "none" }), "1.234");
  assert.equal(format(money(12, "JPY"), { style: "none" }), "12");
});

test('style "symbol" can be given explicitly and an empty options object is fine', () => {
  assert.equal(format(money(123456, "USD"), { style: "symbol" }), "$1,234.56");
  assert.equal(format(money(123456, "USD"), {}), "$1,234.56");
});

test("an unknown style throws", () => {
  assert.throws(() => format(money(1, "USD"), { style: "fancy" }), Error);
});

test("every safe integer is formatted exactly (no floating-point division)", () => {
  assert.equal(format(money(Number.MAX_SAFE_INTEGER, "USD")), "$90,071,992,547,409.91");
  assert.equal(format(money(-Number.MAX_SAFE_INTEGER, "USD")), "-$90,071,992,547,409.91");
  assert.equal(format(money(Number.MAX_SAFE_INTEGER, "JPY")), "¥9,007,199,254,740,991");
  assert.equal(format(money(Number.MAX_SAFE_INTEGER, "KWD")), "KD9,007,199,254,740.991");
  assert.equal(format(money(1234567890123, "USD"), { style: "none" }), "12,345,678,901.23");
});

test("format does not mutate its input and works for registered custom currencies", async () => {
  const lib = await load();
  const m = money(1234, "USD");
  format(m);
  assert.deepEqual(Object.keys(m).sort(), ["amount", "currency"]);
  lib.registerCurrency({ code: "NOK", symbol: "kr", exponent: 2 });
  assert.equal(format(money(123456, "NOK")), "kr1,234.56");
  lib.registerCurrency({ code: "CLP", symbol: "CLP$", exponent: 0 });
  assert.equal(format(money(-2500, "CLP")), "-CLP$2,500");
  lib.registerCurrency({ code: "BHD", symbol: "BD", exponent: 4 });
  assert.equal(format(money(15, "BHD")), "BD0.0015");
});
