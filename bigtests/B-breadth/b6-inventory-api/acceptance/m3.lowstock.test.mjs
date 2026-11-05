import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { boot, makeProduct, move, assertError } from "./_lib.mjs";

let t;
before(async () => {
  t = await boot();
});
after(() => t.stop());

const report = async (api = t.admin) => {
  const r = await api.get("/reports/low-stock");
  assert.equal(r.status, 200);
  return r.body.products;
};

test("the report starts empty", async () => {
  assert.deepEqual(await report(), []);
});

test("products with reorder_level 0 are never listed, even at zero stock", async () => {
  await makeProduct(t.admin, "ls-zero", { reorder_level: 0 });
  assert.deepEqual((await report()).map((p) => p.sku), []);
});

test("a product is low when stock <= reorder_level, including equality", async () => {
  await makeProduct(t.admin, "ls-a", { reorder_level: 5 });
  await move(t.admin, "ls-a", "in", 6);
  assert.ok(!(await report()).some((p) => p.sku === "ls-a"), "6 > 5 is not low");
  await move(t.admin, "ls-a", "out", 1);
  assert.ok((await report()).some((p) => p.sku === "ls-a"), "5 <= 5 is low");
});

test("entries carry the product fields plus shortfall", async () => {
  await makeProduct(t.admin, "ls-b", { name: "Bolt", reorder_level: 10, price_cents: 15 });
  await move(t.admin, "ls-b", "in", 4);
  const e = (await report()).find((p) => p.sku === "ls-b");
  assert.ok(e);
  assert.equal(e.name, "Bolt");
  assert.equal(e.price_cents, 15);
  assert.equal(e.stock, 4);
  assert.equal(e.reorder_level, 10);
  assert.equal(e.shortfall, 6);
  const atLevel = (await report()).find((p) => p.sku === "ls-a");
  assert.equal(atLevel.shortfall, 0);
});

test("sorted by shortfall descending, then sku ascending", async () => {
  await makeProduct(t.admin, "srt-z", { reorder_level: 20 });
  await makeProduct(t.admin, "srt-m", { reorder_level: 20 });
  await makeProduct(t.admin, "srt-b", { reorder_level: 3 });
  await move(t.admin, "srt-z", "in", 5);
  await move(t.admin, "srt-m", "in", 5);
  const list = (await report()).filter((p) => p.sku.startsWith("srt-"));
  assert.deepEqual(list.map((p) => p.sku), ["srt-m", "srt-z", "srt-b"]);
  assert.deepEqual(list.map((p) => p.shortfall), [15, 15, 3]);
  const all = await report();
  const shortfalls = all.map((p) => p.shortfall);
  assert.deepEqual(shortfalls, [...shortfalls].sort((a, b) => b - a));
});

test("the report follows stock movements and reorder_level changes", async () => {
  await makeProduct(t.admin, "dyn-1", { reorder_level: 4 });
  assert.ok((await report()).some((p) => p.sku === "dyn-1"), "stock 0 <= 4");
  await move(t.admin, "dyn-1", "in", 10);
  assert.ok(!(await report()).some((p) => p.sku === "dyn-1"));
  await t.admin.patch("/products/dyn-1", { reorder_level: 12 });
  assert.ok((await report()).some((p) => p.sku === "dyn-1"));
  await t.admin.patch("/products/dyn-1", { reorder_level: 0 });
  assert.ok(!(await report()).some((p) => p.sku === "dyn-1"));
});

test("deleted products leave the report; staff can read it; no key is 401", async () => {
  await makeProduct(t.admin, "del-ls", { reorder_level: 2 });
  assert.ok((await report(t.staff)).some((p) => p.sku === "del-ls"));
  await t.admin.del("/products/del-ls");
  assert.ok(!(await report(t.staff)).some((p) => p.sku === "del-ls"));
  assertError(await t.anon.get("/reports/low-stock"), 401);
});
