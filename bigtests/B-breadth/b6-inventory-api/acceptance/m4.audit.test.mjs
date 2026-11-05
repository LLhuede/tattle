import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { rmSync } from "node:fs";
import { tmpDir } from "../../../common/harness.mjs";
import { boot, makeProduct, move, assertError, ADMIN_KEY, STAFF_KEY } from "./_lib.mjs";

let t;
before(async () => {
  t = await boot();
});
after(() => t.stop());

const entries = async (qs = "") => {
  const r = await t.admin.get(`/audit-log${qs}`);
  assert.equal(r.status, 200);
  return r.body.entries;
};

test("only admin may read the audit log", async () => {
  assertError(await t.staff.get("/audit-log"), 403);
  assertError(await t.anon.get("/audit-log"), 401);
  const r = await t.admin.get("/audit-log");
  assert.equal(r.status, 200);
  assert.ok(Array.isArray(r.body.entries));
});

test("each successful change is logged with actor, action, target and details; newest first", async () => {
  await makeProduct(t.admin, "au-1", { reorder_level: 1 });
  await t.admin.patch("/products/au-1", { price_cents: 99, name: "Renamed" });
  await move(t.staff, "au-1", "in", 8);
  await move(t.admin, "au-1", "out", 3);
  await move(t.admin, "au-1", "out", 5);
  await t.admin.del("/products/au-1");
  const mine = (await entries("?limit=200")).filter((e) => e.target === "au-1");
  assert.deepEqual(mine.map((e) => e.action), ["product.delete", "stock.out", "stock.out", "stock.in", "product.update", "product.create"]);
  assert.deepEqual(mine.map((e) => e.actor), ["alice", "alice", "alice", "bob", "alice", "alice"]);
  assert.deepEqual(mine[1].details, { quantity: 5, stock_after: 0 });
  assert.deepEqual(mine[2].details, { quantity: 3, stock_after: 5 });
  assert.deepEqual(mine[3].details, { quantity: 8, stock_after: 8 });
  assert.deepEqual(mine[4].details, { changed: ["name", "price_cents"] });
  assert.deepEqual(mine[5].details, {});
  assert.deepEqual(mine[0].details, {});
});

test("entries have integer ids (descending in the list), ISO timestamps and no duplicates", async () => {
  const list = await entries("?limit=200");
  assert.ok(list.length >= 6);
  for (const e of list) {
    assert.ok(Number.isInteger(e.id) && e.id >= 1);
    assert.ok(!Number.isNaN(Date.parse(e.at)));
  }
  const ids = list.map((e) => e.id);
  assert.deepEqual(ids, [...ids].sort((a, b) => b - a));
  assert.equal(new Set(ids).size, ids.length);
});

test("refused or failed requests leave no entry, and reads are not logged", async () => {
  await makeProduct(t.admin, "au-2");
  const before = (await entries("?limit=200")).length;
  await t.admin.post("/products", { sku: "au-2", name: "dup" }); // 409
  await t.admin.post("/products", { sku: "bad sku", name: "x" }); // 400
  await t.staff.post("/products", { sku: "au-3", name: "x" }); // 403
  await t.anon.post("/products", { sku: "au-4", name: "x" }); // 401
  await t.admin.patch("/products/nope", { name: "x" }); // 404
  await move(t.admin, "au-2", "out", 1); // 409
  await move(t.admin, "au-2", "in", 0); // 400
  await t.admin.get("/products");
  await t.admin.get("/products/au-2");
  await t.admin.get("/products/au-2/movements");
  await t.admin.get("/reports/low-stock");
  assert.equal((await entries("?limit=200")).length, before);
});

test("limit returns the newest N entries; invalid limits are 400", async () => {
  await makeProduct(t.admin, "lim-1");
  await move(t.admin, "lim-1", "in", 1);
  const two = await entries("?limit=2");
  assert.equal(two.length, 2);
  assert.equal(two[0].action, "stock.in");
  assert.equal(two[1].action, "product.create");
  assert.equal(two[0].target, "lim-1");
  for (const bad of ["0", "201", "abc", "-1", "1.5"]) assertError(await t.admin.get(`/audit-log?limit=${bad}`), 400);
  assert.equal((await entries("?limit=200")).length >= 2, true);
});

test("the default limit is 50", async () => {
  await makeProduct(t.admin, "many-1");
  await move(t.admin, "many-1", "in", 1000);
  for (let i = 0; i < 55; i++) await move(t.admin, "many-1", "out", 1);
  assert.equal((await entries()).length, 50);
  assert.equal((await entries("?limit=200")).length > 50, true);
});

test("action and actor filters match exactly", async () => {
  await makeProduct(t.admin, "fl-1");
  await move(t.staff, "fl-1", "in", 2);
  const staffEntries = await entries("?actor=bob&limit=200");
  assert.ok(staffEntries.length >= 1);
  assert.ok(staffEntries.every((e) => e.actor === "bob"));
  const creates = await entries("?action=product.create&limit=200");
  assert.ok(creates.length >= 1);
  assert.ok(creates.every((e) => e.action === "product.create"));
  const both = await entries("?action=stock.in&actor=bob&limit=200");
  assert.ok(both.some((e) => e.target === "fl-1"));
  assert.deepEqual(await entries("?actor=nobody"), []);
});

test("API key values never appear in the audit log", async () => {
  const r = await t.admin.get("/audit-log?limit=200");
  assert.ok(!r.text.includes(ADMIN_KEY));
  assert.ok(!r.text.includes(STAFF_KEY));
});

test("products, movements, the id counters and the audit log survive a restart", async () => {
  const dataDir = tmpDir("stockroom-persist-");
  const a = await boot({ dataDir });
  let lastMoveId;
  let firstAuditCount;
  try {
    await makeProduct(a.admin, "keep-1", { name: "Persistent", price_cents: 321, reorder_level: 9 });
    await move(a.admin, "keep-1", "in", 7, { note: "kept" });
    const m = await move(a.staff, "keep-1", "out", 2);
    lastMoveId = m.body.id;
    firstAuditCount = (await a.admin.get("/audit-log?limit=200")).body.entries.length;
    assert.equal(firstAuditCount, 3);
  } finally {
    await a.stop();
  }
  const b = await boot({ dataDir });
  try {
    const p = await b.admin.get("/products/keep-1");
    assert.equal(p.status, 200);
    assert.equal(p.body.name, "Persistent");
    assert.equal(p.body.price_cents, 321);
    assert.equal(p.body.stock, 5);
    const hist = (await b.admin.get("/products/keep-1/movements")).body.movements;
    assert.deepEqual(hist.map((m) => m.quantity), [7, 2]);
    assert.equal(hist[0].note, "kept");
    assert.equal(hist[1].actor, "bob");
    const log = (await b.admin.get("/audit-log?limit=200")).body.entries;
    assert.equal(log.length, firstAuditCount);
    const next = await move(b.admin, "keep-1", "in", 1);
    assert.equal(next.body.id, lastMoveId + 1);
    const log2 = (await b.admin.get("/audit-log?limit=200")).body.entries;
    assert.equal(log2.length, firstAuditCount + 1);
    assert.equal(log2[0].id, log[0].id + 1);
    assert.equal((await b.admin.get("/reports/low-stock")).body.products.some((x) => x.sku === "keep-1"), true);
  } finally {
    await b.stop();
    rmSync(dataDir, { recursive: true, force: true });
  }
});
