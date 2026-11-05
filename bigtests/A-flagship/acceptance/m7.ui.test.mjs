import { describe, test, before, after } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { boot } from "./_api.mjs";
import { exists, appPath, importApp } from "../../common/harness.mjs";

describe("m7 browser UI build and money library", () => {
  let s, money;
  before(async () => {
    s = await boot();
    money = await importApp("src/lib/money.js");
  });
  after(async () => {
    await s.stop();
  });

  test("npm run build produces dist/index.html with the title and mount element", () => {
    assert.ok(exists("dist", "index.html"), "dist/index.html is missing");
    const html = readFileSync(appPath("dist", "index.html"), "utf8");
    assert.match(html, /<title>Shared Expenses<\/title>/);
    assert.match(html, /id=["']app["']/);
  });

  test("the server serves dist/index.html at / as text/html", async () => {
    const r = await s.anon.get("/");
    assert.equal(r.status, 200);
    assert.match(r.headers.get("content-type") ?? "", /text\/html/);
    assert.match(r.text, /<title>Shared Expenses<\/title>/);
  });

  test("the script referenced by the page is served", async () => {
    const html = (await s.anon.get("/")).text;
    const m = /<script[^>]*\ssrc=["']([^"']+)["']/.exec(html);
    assert.ok(m, "no <script src> in the page");
    const r = await fetch(new URL(m[1], s.url + "/"));
    assert.equal(r.status, 200);
    const body = await r.text();
    assert.ok(body.length > 100);
    assert.doesNotMatch(r.headers.get("content-type") ?? "", /text\/html/);
  });

  test("the API and health still work next to the UI", async () => {
    assert.equal((await s.anon.get("/health")).status, 200);
    assert.equal((await s.anon.get("/api/me")).status, 401);
  });

  test("money.js exports formatCents and parseAmount", () => {
    assert.equal(typeof money.formatCents, "function");
    assert.equal(typeof money.parseAmount, "function");
  });

  test("formatCents formats zero, cents, thousands and the default currency", () => {
    const { formatCents } = money;
    assert.equal(formatCents(0), "$0.00");
    assert.equal(formatCents(5), "$0.05");
    assert.equal(formatCents(99), "$0.99");
    assert.equal(formatCents(100), "$1.00");
    assert.equal(formatCents(123456), "$1,234.56");
    assert.equal(formatCents(100000000), "$1,000,000.00");
    assert.equal(formatCents(99999, "USD"), "$999.99");
  });

  test("formatCents puts the minus sign before the currency prefix", () => {
    const { formatCents } = money;
    assert.equal(formatCents(-250, "EUR"), "-€2.50");
    assert.equal(formatCents(-1), "-$0.01");
    assert.equal(formatCents(-123456), "-$1,234.56");
    assert.equal(formatCents(-100000, "CAD"), "-CAD 1,000.00");
  });

  test("formatCents knows EUR and GBP and prefixes other codes with the code and a space", () => {
    const { formatCents } = money;
    assert.equal(formatCents(1050, "EUR"), "€10.50");
    assert.equal(formatCents(1050, "GBP"), "£10.50");
    assert.equal(formatCents(1999, "CAD"), "CAD 19.99");
    assert.equal(formatCents(5, "JPY"), "JPY 0.05");
  });

  test("formatCents throws a TypeError for anything but a safe integer", () => {
    const { formatCents } = money;
    for (const bad of [1.5, NaN, Infinity, "100", null, undefined, 2 ** 60]) {
      assert.throws(() => formatCents(bad), TypeError, String(bad));
    }
  });

  test("parseAmount converts whole numbers and one or two decimals exactly", () => {
    const { parseAmount } = money;
    assert.equal(parseAmount("12"), 1200);
    assert.equal(parseAmount("12.5"), 1250);
    assert.equal(parseAmount("12.50"), 1250);
    assert.equal(parseAmount("19.99"), 1999);
    assert.equal(parseAmount("0.99"), 99);
    assert.equal(parseAmount("0.29"), 29);
    assert.equal(parseAmount("1.15"), 115);
    assert.equal(parseAmount("0"), 0);
    assert.equal(parseAmount("0.00"), 0);
    assert.equal(parseAmount("007"), 700);
  });

  test("parseAmount accepts a leading dollar sign, thousands separators and surrounding whitespace", () => {
    const { parseAmount } = money;
    assert.equal(parseAmount("$7"), 700);
    assert.equal(parseAmount("$1,234.56"), 123456);
    assert.equal(parseAmount("1,234,567"), 123456700);
    assert.equal(parseAmount(" 3.10 "), 310);
    assert.equal(parseAmount("\t$5\n"), 500);
  });

  test("parseAmount returns null for invalid input and never throws", () => {
    const { parseAmount } = money;
    const bad = ["1.234", "1,23", "1,2345", "12,34,567", "-5", "+5", ".5", "12.", "abc", "", "   ", "1e3", "$ 5", "$$5", "5$", "1.2.3", "€5", "1 000"];
    for (const b of bad) assert.equal(parseAmount(b), null, JSON.stringify(b));
    for (const b of [5, null, undefined, {}, [], true]) assert.equal(parseAmount(b), null, String(b));
  });

  test("parseAmount returns null when the result is not a safe integer", () => {
    const { parseAmount } = money;
    assert.equal(parseAmount("99999999999999999999"), null);
    assert.equal(parseAmount("90071992547410"), null);
    assert.equal(parseAmount("90071992547409.91"), 9007199254740991);
    assert.equal(parseAmount("1000000000"), 100000000000);
  });

  test("parseAmount and formatCents round-trip", () => {
    const { parseAmount, formatCents } = money;
    for (const c of [0, 1, 99, 100, 1999, 123456, 98765432]) {
      assert.equal(parseAmount(formatCents(c)), c);
    }
  });
});
