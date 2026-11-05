// Requirement changes made during the build (suite.json turns 11 and 13):
//  1. expenses get a `title` field that means the same as `description`; the API stays backwards compatible.
//  2. a single expense may not exceed 100000000 cents.
import { describe, test, before, after } from "node:test";
import assert from "node:assert/strict";
import { boot, newUser, newGroup, addExpense, equalExpense, balancesOf } from "./_api.mjs";

describe("m6 amendments: title field and amount cap", () => {
  let s, ann, bob, gid;
  before(async () => {
    s = await boot();
    ann = await newUser(s, "Ann");
    bob = await newUser(s, "Bob");
    gid = await newGroup(ann, [bob]);
  });
  after(async () => {
    await s.stop();
  });

  const post = (body) => ann.api.post(`/api/groups/${gid}/expenses`, body);
  const split = () => ({ type: "equal", participants: [ann.id, bob.id] });

  test("an expense created with title returns both title and description", async () => {
    const r = await post({ title: "  Groceries ", amountCents: 1000, paidBy: ann.id, split: split() });
    assert.equal(r.status, 201);
    assert.equal(r.body.title, "Groceries");
    assert.equal(r.body.description, "Groceries");
    assert.deepEqual(r.body.shares, { [ann.id]: 500, [bob.id]: 500 });
  });

  test("an expense created with the old description field still works and also has title", async () => {
    const r = await post({ description: "Rent", amountCents: 1000, paidBy: ann.id, split: split() });
    assert.equal(r.status, 201);
    assert.equal(r.body.description, "Rent");
    assert.equal(r.body.title, "Rent");
  });

  test("sending title and description together works when they are equal and is 400 when they differ", async () => {
    const ok = await post({ title: "Same", description: "Same", amountCents: 100, paidBy: ann.id, split: split() });
    assert.equal(ok.status, 201);
    const bad = await post({ title: "One", description: "Two", amountCents: 100, paidBy: ann.id, split: split() });
    assert.equal(bad.status, 400);
    assert.equal(typeof bad.body.error, "string");
  });

  test("title follows the same length rules and one of the two is required", async () => {
    for (const title of ["", "   ", "x".repeat(101), 3]) {
      const r = await post({ title, amountCents: 100, paidBy: ann.id, split: split() });
      assert.equal(r.status, 400, JSON.stringify(title));
    }
    assert.equal((await post({ amountCents: 100, paidBy: ann.id, split: split() })).status, 400);
    assert.equal((await post({ title: "x".repeat(100), amountCents: 100, paidBy: ann.id, split: split() })).status, 201);
  });

  test("get and list return both title and description", async () => {
    const e = (await post({ title: "Fuel", amountCents: 4000, paidBy: bob.id, split: split() })).body;
    const one = await bob.api.get(`/api/groups/${gid}/expenses/${e.id}`);
    assert.equal(one.body.title, "Fuel");
    assert.equal(one.body.description, "Fuel");
    const list = (await bob.api.get(`/api/groups/${gid}/expenses`)).body.expenses;
    const found = list.find((x) => x.id === e.id);
    assert.equal(found.title, "Fuel");
    assert.equal(found.description, "Fuel");
  });

  test("the existing expense fields are all still present (backwards compatible)", async () => {
    const e = await addExpense(ann, gid, equalExpense(ann.id, 600, [ann.id, bob.id], "Compat"));
    for (const k of ["id", "groupId", "description", "amountCents", "paidBy", "split", "shares", "createdBy", "createdAt"]) {
      assert.ok(k in e, `missing ${k}`);
    }
  });

  test("an amount of exactly 100000000 cents is accepted and above it is 400", async () => {
    const g = await newGroup(ann, [bob]);
    const ok = await ann.api.post(`/api/groups/${g}/expenses`, equalExpense(ann.id, 100000000, [ann.id, bob.id]));
    assert.equal(ok.status, 201);
    assert.deepEqual(await balancesOf(ann, g), { [ann.id]: 50000000, [bob.id]: -50000000 });
    for (const amount of [100000001, 250000000]) {
      const r = await ann.api.post(`/api/groups/${g}/expenses`, equalExpense(ann.id, amount, [ann.id, bob.id]));
      assert.equal(r.status, 400, String(amount));
      assert.equal(typeof r.body.error, "string");
    }
  });

  test("the cap applies to exact splits too", async () => {
    const r = await post({ description: "Yacht", amountCents: 100000001, paidBy: ann.id, split: { type: "exact", shares: { [ann.id]: 100000000, [bob.id]: 1 } } });
    assert.equal(r.status, 400);
  });
});
