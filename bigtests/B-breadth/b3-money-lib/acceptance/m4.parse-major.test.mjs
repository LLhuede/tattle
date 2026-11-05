import { test } from "node:test";
import assert from "node:assert/strict";
import { load, isMoney } from "./_api.mjs";

const lib = await load();
const { money, parse, format, fromMajor, toMajor, registerCurrency } = lib;

test("parse reads symbol forms and infers the currency", () => {
  isMoney(parse("$12.50"), 1250, "USD");
  isMoney(parse("€3"), 300, "EUR");
  isMoney(parse("£0.99"), 99, "GBP");
  isMoney(parse("¥500"), 500, "JPY");
  isMoney(parse("CHF1,234.50"), 123450, "CHF");
  isMoney(parse("KD1.234"), 1234, "KWD");
});

test("parse handles a leading minus, whitespace and thousands separators", () => {
  isMoney(parse("-$12.50"), -1250, "USD");
  isMoney(parse("  $1,234,567.89  "), 123456789, "USD");
  isMoney(parse("-1,000", "JPY"), -1000, "JPY");
  isMoney(parse("1234.5", "USD"), 123450, "USD");
  isMoney(parse("0.05", "USD"), 5, "USD");
});

test("fewer decimals than the exponent are fine and more are an error (never rounded)", () => {
  isMoney(parse("5", "USD"), 500, "USD");
  isMoney(parse("5.5", "USD"), 550, "USD");
  isMoney(parse("2.5", "KWD"), 2500, "KWD");
  assert.throws(() => parse("1.234", "USD"), Error);
  assert.throws(() => parse("12.5", "JPY"), Error);
  assert.throws(() => parse("$1.001"), Error);
  isMoney(parse("1.000", "KWD"), 1000, "KWD");
});

test("parse reads codes before or after the number", () => {
  isMoney(parse("12.50 EUR"), 1250, "EUR");
  isMoney(parse("EUR 12.50"), 1250, "EUR");
  isMoney(parse("-5.00 USD"), -500, "USD");
  isMoney(parse("-USD 5"), -500, "USD");
  isMoney(parse("1,500   JPY"), 1500, "JPY");
  isMoney(parse("CHF 10"), 1000, "CHF");
});

test("the currency argument supplies or must agree with the text", () => {
  isMoney(parse("12.50", "GBP"), 1250, "GBP");
  isMoney(parse("$12.50", "USD"), 1250, "USD");
  isMoney(parse("12.50 EUR", "EUR"), 1250, "EUR");
  assert.throws(() => parse("$12.50", "EUR"), Error);
  assert.throws(() => parse("12.50 EUR", "USD"), Error);
  assert.throws(() => parse("12.50"), Error);
  assert.throws(() => parse("12.50", "XXX"), Error);
});

test("malformed text throws", () => {
  for (const bad of ["", "   ", "abc", "$", "$.5", "$5.", "1,23.00", "1,2345.00", "$1,23", "1 000", "$ 5", "$-5", "--5", "5 $", "12.50 XXX", "XXX 12.50", "1e3", "$1.2.3", "+$5", "$5 USD", null, undefined, 12]) {
    assert.throws(() => parse(bad, "USD"), Error, JSON.stringify(bad));
  }
});

test("parse never gives -0 and rejects values that are not safe integers", () => {
  isMoney(parse("-0", "USD"), 0, "USD");
  isMoney(parse("-$0.00"), 0, "USD");
  isMoney(parse("90,071,992,547,409.91", "USD"), Number.MAX_SAFE_INTEGER, "USD");
  assert.throws(() => parse("90,071,992,547,409.92", "USD"), Error);
  assert.throws(() => parse("99999999999999999999", "JPY"), Error);
});

test("parse(format(m)) round-trips", () => {
  for (const m of [money(0, "USD"), money(5, "USD"), money(-123456, "USD"), money(500, "JPY"), money(-1234567, "KWD"), money(123450, "CHF"), money(99, "GBP"), money(Number.MAX_SAFE_INTEGER, "EUR"), money(-Number.MAX_SAFE_INTEGER, "JPY")]) {
    const back = parse(format(m));
    isMoney(back, m.amount, m.currency);
    isMoney(parse(format(m, { style: "code" })), m.amount, m.currency);
    isMoney(parse(format(m, { style: "none" }), m.currency), m.amount, m.currency);
  }
});

test("fromMajor rounds on the decimal the number prints as, half away from zero", () => {
  isMoney(fromMajor(12.34, "USD"), 1234, "USD");
  isMoney(fromMajor(1.005, "USD"), 101, "USD");
  isMoney(fromMajor(-1.005, "USD"), -101, "USD");
  isMoney(fromMajor(0.285, "USD"), 29, "USD");
  isMoney(fromMajor(19.999, "USD"), 2000, "USD");
  isMoney(fromMajor(0.1 + 0.2, "USD"), 30, "USD");
  isMoney(fromMajor(1234.5, "JPY"), 1235, "JPY");
  isMoney(fromMajor(2.0005, "KWD"), 2001, "KWD");
  isMoney(fromMajor(5, "USD"), 500, "USD");
  isMoney(fromMajor(-0.001, "USD"), 0, "USD");
  isMoney(fromMajor(0, "USD"), 0, "USD");
  for (const bad of [NaN, Infinity, "1", null]) assert.throws(() => fromMajor(bad, "USD"), Error, String(bad));
  assert.throws(() => fromMajor(1, "XXX"), Error);
  assert.throws(() => fromMajor(1e20, "USD"), Error);
});

test("toMajor divides by 10^exponent", () => {
  assert.equal(toMajor(money(1234, "USD")), 12.34);
  assert.equal(toMajor(money(-5, "USD")), -0.05);
  assert.equal(toMajor(money(1234, "KWD")), 1.234);
  assert.equal(toMajor(money(1234, "JPY")), 1234);
  assert.equal(toMajor(money(0, "USD")), 0);
});

test("custom currencies work in parse, fromMajor and toMajor", () => {
  registerCurrency({ code: "NOK", symbol: "kr", exponent: 2 });
  isMoney(parse("kr1,234.56"), 123456, "NOK");
  isMoney(parse("10 NOK"), 1000, "NOK");
  isMoney(fromMajor(1.005, "NOK"), 101, "NOK");
  assert.equal(toMajor(money(250, "NOK")), 2.5);
  registerCurrency({ code: "CLP", symbol: "CLP$", exponent: 0 });
  isMoney(parse("-CLP$2,500"), -2500, "CLP");
});

test("a symbol shared by two currencies is ambiguous unless the currency argument is one of them", () => {
  registerCurrency({ code: "CAD", symbol: "$", exponent: 2 });
  assert.throws(() => parse("$5.00"), Error);
  isMoney(parse("$5.00", "CAD"), 500, "CAD");
  isMoney(parse("$5.00", "USD"), 500, "USD");
  assert.throws(() => parse("$5.00", "EUR"), Error);
  isMoney(parse("5.00 CAD"), 500, "CAD");
  isMoney(parse("€5.00"), 500, "EUR");
});
