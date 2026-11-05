"use strict";

const registry = new Map();
function addCurrency(c) {
  registry.set(c.code, Object.freeze({ ...c }));
  return registry.get(c.code);
}
[
  ["USD", "$", 2, "US Dollar"],
  ["EUR", "€", 2, "Euro"],
  ["GBP", "£", 2, "Pound Sterling"],
  ["JPY", "¥", 0, "Yen"],
  ["CHF", "CHF", 2, "Swiss Franc"],
  ["KWD", "KD", 3, "Kuwaiti Dinar"],
].forEach(([code, symbol, exponent, name]) => addCurrency({ code, symbol, exponent, name }));

function getCurrency(code) {
  return registry.get(code);
}
function listCurrencies() {
  return [...registry.values()].sort((a, b) => (a.code < b.code ? -1 : 1)).map((c) => ({ ...c }));
}
function registerCurrency(c) {
  if (!c || typeof c !== "object") throw new Error("currency must be an object");
  if (typeof c.code !== "string" || !/^[A-Z]{3}$/.test(c.code)) throw new Error("bad code");
  if (typeof c.symbol !== "string" || !c.symbol) throw new Error("bad symbol");
  if (!Number.isInteger(c.exponent) || c.exponent < 0 || c.exponent > 6) throw new Error("bad exponent");
  if (c.name !== undefined && typeof c.name !== "string") throw new Error("bad name");
  if (registry.has(c.code)) throw new Error("already registered");
  return { ...addCurrency({ code: c.code, symbol: c.symbol, exponent: c.exponent, name: c.name ?? c.code }) };
}
function cur(code) {
  const c = typeof code === "string" ? registry.get(code) : undefined;
  if (!c) throw new Error(`unknown currency ${String(code)}`);
  return c;
}

function checkAmount(n) {
  if (!Number.isSafeInteger(n)) throw new Error(`not a safe integer: ${n}`);
  return n === 0 ? 0 : n;
}
function money(amount, currency) {
  cur(currency);
  return { amount: checkAmount(amount), currency };
}
function same(a, b) {
  if (a.currency !== b.currency) throw new Error("currency mismatch");
}
const add = (a, b) => (same(a, b), money(a.amount + b.amount, a.currency));
const subtract = (a, b) => (same(a, b), money(a.amount - b.amount, a.currency));
const negate = (m) => money(-m.amount, m.currency);
const compare = (a, b) => (same(a, b), a.amount < b.amount ? -1 : a.amount > b.amount ? 1 : 0);
const equals = (a, b) => a.currency === b.currency && a.amount === b.amount;

function roundDiv(num, den) {
  const neg = num < 0n;
  const n = neg ? -num : num;
  let q = n / den;
  if (2n * (n % den) >= den) q += 1n;
  return neg ? -q : q;
}
function toSafe(big) {
  if (big > BigInt(Number.MAX_SAFE_INTEGER) || big < -BigInt(Number.MAX_SAFE_INTEGER)) throw new Error("result out of range");
  return Number(big);
}
// value (a finite number, taken as String(value)) * mult, scaled by 10^shift, rounded half away from zero
function decimal(value) {
  if (typeof value !== "number" || !Number.isFinite(value)) throw new Error("value must be a finite number");
  const m = /^(-?)(\d+)(?:\.(\d+))?(?:e([+-]?\d+))?$/.exec(String(value));
  const frac = m[3] || "";
  const digits = BigInt(m[2] + frac) * (m[1] ? -1n : 1n);
  return { digits, scale: frac.length - Number(m[4] || 0) };
}
function roundScaled(value, mult, shift) {
  const { digits, scale } = decimal(value);
  let num = digits * mult;
  let s = scale - shift; // value * 10^shift = num / 10^s
  if (s <= 0) return num * 10n ** BigInt(-s);
  return roundDiv(num, 10n ** BigInt(s));
}

function multiply(m, factor) {
  return money(toSafe(roundScaled(factor, BigInt(m.amount), 0)), m.currency);
}
function allocate(m, ratios) {
  if (!Array.isArray(ratios) || !ratios.length || !ratios.every((r) => Number.isSafeInteger(r) && r >= 0)) throw new Error("bad ratios");
  const total = ratios.reduce((a, b) => a + BigInt(b), 0n);
  if (total <= 0n) throw new Error("ratios must have a positive sum");
  const neg = m.amount < 0;
  const abs = BigInt(Math.abs(m.amount));
  const parts = ratios.map((r) => (abs * BigInt(r)) / total);
  let left = abs - parts.reduce((a, b) => a + b, 0n);
  for (let i = 0; i < parts.length && left > 0n; i++) {
    if (ratios[i] === 0) continue;
    parts[i] += 1n;
    left -= 1n;
  }
  return parts.map((p) => money(neg ? -Number(p) : Number(p), m.currency));
}
function split(m, n) {
  if (!Number.isSafeInteger(n) || n <= 0) throw new Error("n must be a positive integer");
  return allocate(m, new Array(n).fill(1));
}

function format(m, options = {}) {
  const c = cur(m.currency);
  const style = options.style ?? "symbol";
  if (!["symbol", "code", "none"].includes(style)) throw new Error("bad style");
  const neg = m.amount < 0;
  let digits = String(Math.abs(m.amount)).padStart(c.exponent + 1, "0");
  let int = digits.slice(0, digits.length - c.exponent);
  const frac = digits.slice(digits.length - c.exponent);
  int = int.replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const num = c.exponent ? `${int}.${frac}` : int;
  const sign = neg ? "-" : "";
  if (style === "code") return `${sign}${num} ${c.code}`;
  if (style === "none") return `${sign}${num}`;
  return `${sign}${c.symbol}${num}`;
}

const NUMBER = "(\\d+|\\d{1,3}(?:,\\d{3})+)(?:\\.(\\d+))?";
function parse(text, currency) {
  if (typeof text !== "string") throw new Error("text must be a string");
  let s = text.trim();
  let neg = false;
  if (s.startsWith("-")) {
    neg = true;
    s = s.slice(1);
  }
  let named; // candidate currencies from the text
  let numText;
  const num = new RegExp(`^${NUMBER}$`);
  let m;
  if ((m = new RegExp(`^(?:([A-Z]{3})\\s+)${NUMBER}$`).exec(s)) && registry.has(m[1])) {
    named = [registry.get(m[1])];
    numText = s.replace(/^[A-Z]{3}\s+/, "");
  } else if ((m = new RegExp(`^${NUMBER}\\s+([A-Z]{3})$`).exec(s)) && registry.has(m[3])) {
    named = [registry.get(m[3])];
    numText = s.replace(/\s+[A-Z]{3}$/, "");
  } else if (num.test(s)) {
    numText = s;
  } else {
    const sym = [...registry.values()].filter((c) => s.startsWith(c.symbol) && num.test(s.slice(c.symbol.length)));
    if (!sym.length) throw new Error(`cannot parse money: ${text}`);
    const symbol = sym[0].symbol;
    numText = s.slice(symbol.length);
    named = sym;
    if (sym.length > 1) {
      if (currency === undefined || !sym.some((c) => c.code === currency)) throw new Error("ambiguous symbol");
      named = [registry.get(currency)];
    }
  }
  let c;
  if (named) {
    c = named[0];
    if (currency !== undefined && currency !== c.code) throw new Error("currency mismatch");
  } else {
    if (currency === undefined) throw new Error("no currency given");
    c = cur(currency);
  }
  const nm = num.exec(numText);
  const int = nm[1].replace(/,/g, "");
  const frac = nm[2] || "";
  if (frac.length > c.exponent) throw new Error("too many decimal digits");
  const minor = BigInt(int + frac.padEnd(c.exponent, "0"));
  return { amount: toSafe(neg ? -minor : minor) || 0, currency: c.code };
}

function fromMajor(value, currency) {
  const c = cur(currency);
  return money(toSafe(roundScaled(value, 1n, c.exponent)), c.code);
}
function toMajor(m) {
  return m.amount / 10 ** cur(m.currency).exponent;
}

module.exports = { getCurrency, listCurrencies, registerCurrency, money, add, subtract, negate, compare, equals, multiply, allocate, split, format, parse, fromMajor, toMajor };
