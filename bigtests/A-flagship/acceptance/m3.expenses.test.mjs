import { describe, test, before, after } from "node:test";
import assert from "node:assert/strict";
import { boot, newUser, newGroup, addExpense, equalExpense } from "./_api.mjs";

describe("m3 expenses and splits", () => {
  let s, ann, bob, cy, outsider, gid;
  before(async () => {
    s = await boot();
    ann = await newUser(s, "Ann");
    bob = await newUser(s, "Bob");
    cy = await newUser(s, "Cy");
    outsider = await newUser(s, "Out");
    gid = await newGroup(ann, [bob, cy]);
  });
  after(async () => {
    await s.stop();
  });

  const post = (user, body, group = gid) => user.api.post(`/api/groups/${group}/expenses`, body);

  test("equal split with a remainder gives the extra cents to the first participants in order", async () => {
    const e = await addExpense(ann, gid, equalExpense(ann.id, 1000, [ann.id, bob.id, cy.id]));
    assert.deepEqual(e.shares, { [ann.id]: 334, [bob.id]: 333, [cy.id]: 333 });
    const e2 = await addExpense(ann, gid, equalExpense(ann.id, 1000, [cy.id, bob.id, ann.id]));
    assert.deepEqual(e2.shares, { [cy.id]: 334, [bob.id]: 333, [ann.id]: 333 });
    const e3 = await addExpense(ann, gid, equalExpense(bob.id, 101, [ann.id, bob.id, cy.id]));
    assert.deepEqual(e3.shares, { [ann.id]: 34, [bob.id]: 34, [cy.id]: 33 });
  });

  test("equal split without a remainder and with a single participant", async () => {
    const e = await addExpense(ann, gid, equalExpense(ann.id, 900, [ann.id, bob.id, cy.id]));
    assert.deepEqual(e.shares, { [ann.id]: 300, [bob.id]: 300, [cy.id]: 300 });
    const one = await addExpense(ann, gid, equalExpense(ann.id, 777, [bob.id]));
    assert.deepEqual(one.shares, { [bob.id]: 777 });
  });

  test("the created expense has the pinned shape", async () => {
    const body = { description: "  Taxi  ", amountCents: 2500, paidBy: bob.id, split: { type: "equal", participants: [ann.id, bob.id] } };
    const r = await post(ann, body);
    assert.equal(r.status, 201);
    const e = r.body;
    assert.equal(typeof e.id, "string");
    assert.equal(e.groupId, gid);
    assert.equal(e.description, "Taxi");
    assert.equal(e.amountCents, 2500);
    assert.equal(e.paidBy, bob.id);
    assert.equal(e.createdBy, ann.id);
    assert.ok(!Number.isNaN(Date.parse(e.createdAt)));
    assert.deepEqual(e.split, { type: "equal", participants: [ann.id, bob.id] });
    assert.deepEqual(e.shares, { [ann.id]: 1250, [bob.id]: 1250 });
  });

  test("exact split stores the shares and echoes the split", async () => {
    const split = { type: "exact", shares: { [ann.id]: 400, [cy.id]: 600 } };
    const r = await post(bob, { description: "Pizza", amountCents: 1000, paidBy: cy.id, split });
    assert.equal(r.status, 201);
    assert.deepEqual(r.body.split, split);
    assert.deepEqual(r.body.shares, { [ann.id]: 400, [cy.id]: 600 });
    assert.equal(r.body.createdBy, bob.id);
  });

  test("exact split whose shares do not sum to the amount is 400", async () => {
    for (const total of [999, 1001]) {
      const r = await post(ann, { description: "x", amountCents: total, paidBy: ann.id, split: { type: "exact", shares: { [ann.id]: 400, [bob.id]: 600 } } });
      assert.equal(r.status, 400, `total ${total}`);
    }
  });

  test("exact split rejects non-members, zero and non-integer shares and an empty object", async () => {
    const bads = [
      { [outsider.id]: 1000 },
      { [ann.id]: 0, [bob.id]: 1000 },
      { [ann.id]: 500.5, [bob.id]: 499.5 },
      { [ann.id]: "500", [bob.id]: 500 },
      {},
    ];
    for (const shares of bads) {
      const r = await post(ann, { description: "x", amountCents: 1000, paidBy: ann.id, split: { type: "exact", shares } });
      assert.equal(r.status, 400, JSON.stringify(shares));
    }
  });

  test("the payer need not be a participant", async () => {
    const r = await post(ann, equalExpense(cy.id, 600, [ann.id, bob.id]));
    assert.equal(r.status, 201);
    assert.equal(r.body.paidBy, cy.id);
  });

  test("amountCents must be a positive integer", async () => {
    for (const amountCents of [0, -5, 10.5, "1000", null, undefined, true]) {
      const r = await post(ann, { ...equalExpense(ann.id, 1, [ann.id]), amountCents });
      assert.equal(r.status, 400, JSON.stringify(amountCents));
      assert.equal(typeof r.body.error, "string");
    }
  });

  test("description must be 1 to 100 characters after trimming", async () => {
    for (const description of ["", "   ", "x".repeat(101), 5, undefined]) {
      const r = await post(ann, { ...equalExpense(ann.id, 100, [ann.id]), description });
      assert.equal(r.status, 400, JSON.stringify(description));
    }
    assert.equal((await post(ann, { ...equalExpense(ann.id, 100, [ann.id]), description: "x".repeat(100) })).status, 201);
  });

  test("paidBy and participants must be group members, participants distinct and non-empty", async () => {
    assert.equal((await post(ann, equalExpense(outsider.id, 100, [ann.id]))).status, 400);
    assert.equal((await post(ann, equalExpense(ann.id, 100, [outsider.id]))).status, 400);
    assert.equal((await post(ann, equalExpense(ann.id, 100, [ann.id, ann.id]))).status, 400);
    assert.equal((await post(ann, equalExpense(ann.id, 100, []))).status, 400);
    assert.equal((await post(ann, { ...equalExpense(ann.id, 100, [ann.id]), paidBy: undefined })).status, 400);
  });

  test("unknown or missing split type is 400", async () => {
    assert.equal((await post(ann, { description: "x", amountCents: 100, paidBy: ann.id, split: { type: "percent", shares: { [ann.id]: 100 } } })).status, 400);
    assert.equal((await post(ann, { description: "x", amountCents: 100, paidBy: ann.id })).status, 400);
  });

  test("check order: 401 before 404 before 403 before 400", async () => {
    const valid = equalExpense(ann.id, 100, [ann.id]);
    assert.equal((await s.anon.post(`/api/groups/${gid}/expenses`, valid)).status, 401);
    assert.equal((await ann.api.post(`/api/groups/nope/expenses`, valid)).status, 404);
    assert.equal((await outsider.api.post(`/api/groups/${gid}/expenses`, { nonsense: true })).status, 403);
    assert.equal((await ann.api.post(`/api/groups/${gid}/expenses`, { nonsense: true })).status, 400);
  });

  test("listing expenses is oldest first and get returns one expense", async () => {
    const g = await newGroup(ann, [bob]);
    const a = await addExpense(ann, g, equalExpense(ann.id, 100, [ann.id, bob.id], "first"));
    const b = await addExpense(bob, g, equalExpense(bob.id, 200, [ann.id, bob.id], "second"));
    const c = await addExpense(ann, g, equalExpense(ann.id, 300, [ann.id, bob.id], "third"));
    const list = await ann.api.get(`/api/groups/${g}/expenses`);
    assert.equal(list.status, 200);
    assert.deepEqual(list.body.expenses.map((e) => e.id), [a.id, b.id, c.id]);
    const one = await bob.api.get(`/api/groups/${g}/expenses/${b.id}`);
    assert.equal(one.status, 200);
    assert.equal(one.body.description, "second");
    assert.equal((await bob.api.get(`/api/groups/${g}/expenses/nope`)).status, 404);
    assert.equal((await outsider.api.get(`/api/groups/${g}/expenses`)).status, 403);
  });

  test("only the creator can delete an expense, with 204 and no body", async () => {
    const g = await newGroup(ann, [bob]);
    const e = await addExpense(ann, g, equalExpense(ann.id, 100, [ann.id, bob.id]));
    const denied = await bob.api.del(`/api/groups/${g}/expenses/${e.id}`);
    assert.equal(denied.status, 403);
    const ok = await ann.api.del(`/api/groups/${g}/expenses/${e.id}`);
    assert.equal(ok.status, 204);
    assert.equal(ok.text, "");
    assert.equal((await ann.api.get(`/api/groups/${g}/expenses/${e.id}`)).status, 404);
    assert.equal((await ann.api.del(`/api/groups/${g}/expenses/${e.id}`)).status, 404);
    assert.deepEqual((await ann.api.get(`/api/groups/${g}/expenses`)).body, { expenses: [] });
  });
});
