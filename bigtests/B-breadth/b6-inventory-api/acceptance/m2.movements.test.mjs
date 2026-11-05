import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { boot, makeProduct, move, assertError } from "./_lib.mjs";

let t;
before(async () => {
  t = await boot();
});
after(() => t.stop());

test("an 'in' movement adds stock and returns the movement record", async () => {
  await makeProduct(t.admin, "mv-1");
  const r = await move(t.admin, "mv-1", "in", 10, { note: "first delivery" });
  assert.equal(r.status, 201);
  assert.equal(typeof r.body.id, "number");
  assert.equal(r.body.sku, "mv-1");
  assert.equal(r.body.type, "in");
  assert.equal(r.body.quantity, 10);
  assert.equal(r.body.note, "first delivery");
  assert.equal(r.body.actor, "alice");
  assert.equal(r.body.stock_after, 10);
  assert.ok(!Number.isNaN(Date.parse(r.body.created_at)));
  assert.equal((await t.admin.get("/products/mv-1")).body.stock, 10);
});

test("an 'out' movement subtracts; note is null when not given", async () => {
  await makeProduct(t.admin, "mv-2");
  await move(t.admin, "mv-2", "in", 10);
  const r = await move(t.admin, "mv-2", "out", 4);
  assert.equal(r.status, 201);
  assert.equal(r.body.note, null);
  assert.equal(r.body.stock_after, 6);
  assert.equal((await t.admin.get("/products/mv-2")).body.stock, 6);
});

test("an 'out' bigger than the stock is 409 and changes nothing; taking exactly all is fine", async () => {
  await makeProduct(t.admin, "mv-3");
  await move(t.admin, "mv-3", "in", 5);
  assertError(await move(t.admin, "mv-3", "out", 6), 409);
  assert.equal((await t.admin.get("/products/mv-3")).body.stock, 5);
  const all = await move(t.admin, "mv-3", "out", 5);
  assert.equal(all.status, 201);
  assert.equal(all.body.stock_after, 0);
  assertError(await move(t.admin, "mv-3", "out", 1), 409);
  const hist = await t.admin.get("/products/mv-3/movements");
  assert.equal(hist.body.movements.length, 2, "refused movements are not recorded");
});

test("movement validation gives 400; unknown sku gives 404", async () => {
  await makeProduct(t.admin, "mv-4");
  const bads = [
    {},
    { type: "in" },
    { quantity: 5 },
    { type: "sideways", quantity: 5 },
    { type: "in", quantity: 0 },
    { type: "in", quantity: -3 },
    { type: "in", quantity: 1.5 },
    { type: "in", quantity: "5" },
    { type: "in", quantity: 1000001 },
    { type: "in", quantity: 1, note: 42 },
    { type: "in", quantity: 1, note: "n".repeat(201) },
  ];
  for (const b of bads) assertError(await t.admin.post("/products/mv-4/movements", b), 400);
  assertError(await t.admin.post("/products/mv-4/movements", "garbage"), 400);
  assert.equal((await t.admin.get("/products/mv-4")).body.stock, 0);
  assertError(await move(t.admin, "no-such-sku", "in", 1), 404);
  const edge = await move(t.admin, "mv-4", "in", 1000000, { note: "n".repeat(200) });
  assert.equal(edge.status, 201);
});

test("staff may record movements and read them; the actor is the key's name", async () => {
  await makeProduct(t.admin, "mv-5");
  const r = await move(t.staff, "mv-5", "in", 3);
  assert.equal(r.status, 201);
  assert.equal(r.body.actor, "bob");
  const h = await t.staff.get("/products/mv-5/movements");
  assert.equal(h.status, 200);
  assert.equal(h.body.movements.length, 1);
  assert.equal(h.body.movements[0].actor, "bob");
});

test("movement ids come from one shared counter and the history is in ascending id order", async () => {
  await makeProduct(t.admin, "id-a");
  await makeProduct(t.admin, "id-b");
  const m1 = await move(t.admin, "id-a", "in", 5);
  const m2 = await move(t.admin, "id-b", "in", 5);
  const m3 = await move(t.admin, "id-a", "out", 1);
  assert.ok(m2.body.id > m1.body.id);
  assert.ok(m3.body.id > m2.body.id);
  assert.equal(m2.body.id, m1.body.id + 1);
  assert.equal(m3.body.id, m2.body.id + 1);
  const h = (await t.admin.get("/products/id-a/movements")).body.movements;
  assert.deepEqual(h.map((m) => m.id), [m1.body.id, m3.body.id]);
  assert.deepEqual(h.map((m) => m.type), ["in", "out"]);
});

test("the movement history can be filtered by type; other values are 400", async () => {
  await makeProduct(t.admin, "flt-1");
  await move(t.admin, "flt-1", "in", 10);
  await move(t.admin, "flt-1", "out", 3);
  await move(t.admin, "flt-1", "in", 2);
  const ins = (await t.admin.get("/products/flt-1/movements?type=in")).body.movements;
  assert.deepEqual(ins.map((m) => m.quantity), [10, 2]);
  const outs = (await t.admin.get("/products/flt-1/movements?type=out")).body.movements;
  assert.deepEqual(outs.map((m) => m.quantity), [3]);
  assertError(await t.admin.get("/products/flt-1/movements?type=sideways"), 400);
});

test("movement routes need a key and a known product", async () => {
  assertError(await t.anon.post("/products/mv-1/movements", { type: "in", quantity: 1 }), 401);
  assertError(await t.anon.get("/products/mv-1/movements"), 401);
  assertError(await t.admin.get("/products/not-a-product/movements"), 404);
});

test("concurrent 'out' requests can never take the stock below zero", async () => {
  await makeProduct(t.admin, "race-1");
  await move(t.admin, "race-1", "in", 5);
  const results = await Promise.all(Array.from({ length: 12 }, () => move(t.admin, "race-1", "out", 1)));
  const ok = results.filter((r) => r.status === 201).length;
  const refused = results.filter((r) => r.status === 409).length;
  assert.equal(ok, 5);
  assert.equal(refused, 7);
  assert.equal((await t.admin.get("/products/race-1")).body.stock, 0);
  const stocks = results.filter((r) => r.status === 201).map((r) => r.body.stock_after).sort();
  assert.deepEqual(stocks, [0, 1, 2, 3, 4]);
});

test("a product with stock cannot be deleted (409); once empty it can, and a re-created sku starts fresh", async () => {
  await makeProduct(t.admin, "gone-1");
  const first = await move(t.admin, "gone-1", "in", 2);
  assertError(await t.admin.del("/products/gone-1"), 409);
  assert.equal((await t.admin.get("/products/gone-1")).status, 200);
  await move(t.admin, "gone-1", "out", 2);
  assert.equal((await t.admin.del("/products/gone-1")).status, 204);
  assertError(await t.admin.get("/products/gone-1/movements"), 404);
  await makeProduct(t.admin, "gone-1");
  assert.equal((await t.admin.get("/products/gone-1")).body.stock, 0);
  assert.deepEqual((await t.admin.get("/products/gone-1/movements")).body.movements, []);
  const next = await move(t.admin, "gone-1", "in", 1);
  assert.ok(next.body.id > first.body.id + 1, "ids are never reused");
});
