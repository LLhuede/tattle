import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { boot, makeProduct, assertError } from "./_lib.mjs";

let t;
before(async () => {
  t = await boot();
});
after(() => t.stop());

test("GET /health is open and returns ok", async () => {
  const r = await t.anon.get("/health");
  assert.equal(r.status, 200);
  assert.deepEqual(r.body, { status: "ok" });
});

test("requests without a key or with a wrong key get 401 {error}", async () => {
  assertError(await t.anon.get("/products"), 401);
  assertError(await t.anon.get("/products", { "x-api-key": "nope" }), 401);
  assertError(await t.anon.post("/products", { sku: "BAD KEY", name: "" }), 401);
});

test("staff cannot create, update or delete products (403); 403 comes before 400 and 404", async () => {
  assertError(await t.staff.post("/products", { sku: "S-1", name: "x" }), 403);
  assertError(await t.staff.post("/products", {}), 403);
  assertError(await t.staff.patch("/products/NOPE", { name: "y" }), 403);
  assertError(await t.staff.del("/products/NOPE"), 403);
});

test("admin creates a product: 201, defaults, stock 0, ISO timestamps", async () => {
  const r = await t.admin.post("/products", { sku: "WID-1", name: "Widget" });
  assert.equal(r.status, 201);
  assert.equal(r.body.sku, "WID-1");
  assert.equal(r.body.name, "Widget");
  assert.equal(r.body.price_cents, 0);
  assert.equal(r.body.reorder_level, 0);
  assert.equal(r.body.stock, 0);
  assert.ok(!Number.isNaN(Date.parse(r.body.created_at)));
  assert.ok(!Number.isNaN(Date.parse(r.body.updated_at)));
  const full = await t.admin.post("/products", { sku: "WID-2", name: "Gadget", price_cents: 1999, reorder_level: 7, stock: 500 });
  assert.equal(full.status, 201);
  assert.equal(full.body.price_cents, 1999);
  assert.equal(full.body.reorder_level, 7);
  assert.equal(full.body.stock, 0, "a stock field in the body is ignored");
});

test("create validation: bad sku, name, price_cents, reorder_level and bodies give 400 {error}", async () => {
  const bads = [
    {},
    { name: "no sku" },
    { sku: "", name: "x" },
    { sku: "has space", name: "x" },
    { sku: "a".repeat(33), name: "x" },
    { sku: 123, name: "x" },
    { sku: "V-1" },
    { sku: "V-2", name: "   " },
    { sku: "V-3", name: "x".repeat(101) },
    { sku: "V-4", name: "x", price_cents: -1 },
    { sku: "V-5", name: "x", price_cents: 1.5 },
    { sku: "V-6", name: "x", price_cents: "10" },
    { sku: "V-7", name: "x", reorder_level: -2 },
    { sku: "V-8", name: "x", reorder_level: 2.5 },
  ];
  for (const b of bads) assertError(await t.admin.post("/products", b), 400);
  assertError(await t.admin.post("/products", "{not json"), 400);
  assertError(await t.admin.post("/products", "[1,2]"), 400);
  const list = await t.admin.get("/products?q=V-");
  assert.deepEqual(list.body.products, []);
});

test("a duplicate sku is 409, but skus are case-sensitive", async () => {
  await makeProduct(t.admin, "dup-1");
  assertError(await t.admin.post("/products", { sku: "dup-1", name: "again" }), 409);
  const upper = await t.admin.post("/products", { sku: "DUP-1", name: "upper" });
  assert.equal(upper.status, 201);
});

test("GET /products/:sku returns the product (staff too) or 404", async () => {
  await makeProduct(t.admin, "get-1", { price_cents: 5 });
  const a = await t.admin.get("/products/get-1");
  assert.equal(a.status, 200);
  assert.equal(a.body.price_cents, 5);
  const s = await t.staff.get("/products/get-1");
  assert.equal(s.status, 200);
  assert.equal(s.body.sku, "get-1");
  assertError(await t.admin.get("/products/does-not-exist"), 404);
});

test("GET /products is sorted by sku and q filters sku or name case-insensitively", async () => {
  for (const [sku, name] of [["lst-c", "Cherry pie"], ["lst-a", "Apple pie"], ["lst-b", "Banana split"], ["zzz-9", "Lstring"]]) {
    await makeProduct(t.admin, sku, { name });
  }
  const all = await t.staff.get("/products");
  assert.equal(all.status, 200);
  const skus = all.body.products.map((p) => p.sku);
  assert.deepEqual(skus, [...skus].sort());
  const q = await t.staff.get("/products?q=PIE");
  assert.deepEqual(q.body.products.map((p) => p.sku), ["lst-a", "lst-c"]);
  const bySku = await t.staff.get("/products?q=lst-");
  assert.deepEqual(bySku.body.products.map((p) => p.sku), ["lst-a", "lst-b", "lst-c"]);
  const byName = await t.staff.get("/products?q=lstr");
  assert.deepEqual(byName.body.products.map((p) => p.sku), ["zzz-9"]);
});

test("PATCH updates name, price_cents and reorder_level and returns the product", async () => {
  const p = await makeProduct(t.admin, "pat-1", { name: "Old", price_cents: 100, reorder_level: 1 });
  const r = await t.admin.patch("/products/pat-1", { name: "New", price_cents: 250 });
  assert.equal(r.status, 200);
  assert.equal(r.body.name, "New");
  assert.equal(r.body.price_cents, 250);
  assert.equal(r.body.reorder_level, 1);
  assert.ok(Date.parse(r.body.updated_at) >= Date.parse(p.updated_at));
  const again = await t.admin.get("/products/pat-1");
  assert.equal(again.body.name, "New");
  const lvl = await t.admin.patch("/products/pat-1", { reorder_level: 0 });
  assert.equal(lvl.body.reorder_level, 0);
});

test("PATCH rejects sku/stock, empty bodies and invalid values with 400; unknown sku is 404", async () => {
  await makeProduct(t.admin, "pat-2", { name: "Keep" });
  assertError(await t.admin.patch("/products/pat-2", { sku: "other" }), 400);
  assertError(await t.admin.patch("/products/pat-2", { stock: 5 }), 400);
  assertError(await t.admin.patch("/products/pat-2", { name: "ok", stock: 5 }), 400);
  assertError(await t.admin.patch("/products/pat-2", {}), 400);
  assertError(await t.admin.patch("/products/pat-2", { name: "" }), 400);
  assertError(await t.admin.patch("/products/pat-2", { price_cents: -5 }), 400);
  assertError(await t.admin.patch("/products/pat-2", { reorder_level: "x" }), 400);
  assertError(await t.admin.patch("/products/nope-404", { name: "x" }), 404);
  assert.equal((await t.admin.get("/products/pat-2")).body.name, "Keep");
});

test("DELETE removes a product with no stock: 204 then 404", async () => {
  await makeProduct(t.admin, "del-1");
  const r = await t.admin.del("/products/del-1");
  assert.equal(r.status, 204);
  assert.equal(r.text, "");
  assertError(await t.admin.get("/products/del-1"), 404);
  assertError(await t.admin.del("/products/del-1"), 404);
});
