import { describe, test, before, after } from "node:test";
import assert from "node:assert/strict";
import { boot, newUser, newGroup, addExpense, equalExpense, balancesOf } from "./_api.mjs";

describe("m5 settlements and activity feed", () => {
  let s, ann, bob, cy, outsider;
  before(async () => {
    s = await boot();
    ann = await newUser(s, "Ann");
    bob = await newUser(s, "Bob");
    cy = await newUser(s, "Cy");
    outsider = await newUser(s, "Out");
  });
  after(async () => {
    await s.stop();
  });

  const settle = (user, g, body) => user.api.post(`/api/groups/${g}/settlements`, body);

  test("recording a settlement returns 201 with the pinned shape", async () => {
    const g = await newGroup(ann, [bob]);
    const r = await settle(bob, g, { from: bob.id, to: ann.id, amountCents: 250 });
    assert.equal(r.status, 201);
    assert.equal(typeof r.body.id, "string");
    assert.equal(r.body.groupId, g);
    assert.equal(r.body.from, bob.id);
    assert.equal(r.body.to, ann.id);
    assert.equal(r.body.amountCents, 250);
    assert.equal(r.body.createdBy, bob.id);
    assert.ok(!Number.isNaN(Date.parse(r.body.createdAt)));
  });

  test("a settlement moves balances toward zero", async () => {
    const g = await newGroup(ann, [bob, cy]);
    await addExpense(ann, g, equalExpense(ann.id, 3000, [ann.id, bob.id, cy.id]));
    assert.equal((await settle(bob, g, { from: bob.id, to: ann.id, amountCents: 1000 })).status, 201);
    assert.deepEqual(await balancesOf(ann, g), { [ann.id]: 1000, [bob.id]: 0, [cy.id]: -1000 });
    const t = await ann.api.get(`/api/groups/${g}/settle-up`);
    assert.deepEqual(t.body.transfers, [{ from: cy.id, to: ann.id, amountCents: 1000 }]);
    assert.equal((await settle(cy, g, { from: cy.id, to: ann.id, amountCents: 1000 })).status, 201);
    assert.deepEqual(await balancesOf(ann, g), { [ann.id]: 0, [bob.id]: 0, [cy.id]: 0 });
    assert.deepEqual((await ann.api.get(`/api/groups/${g}/settle-up`)).body, { transfers: [] });
  });

  test("a settlement can overpay and flip a balance", async () => {
    const g = await newGroup(ann, [bob]);
    await settle(ann, g, { from: ann.id, to: bob.id, amountCents: 500 });
    assert.deepEqual(await balancesOf(ann, g), { [ann.id]: 500, [bob.id]: -500 });
  });

  test("settlement validation returns 400", async () => {
    const g = await newGroup(ann, [bob]);
    const bads = [
      { from: ann.id, to: ann.id, amountCents: 100 },
      { from: ann.id, to: outsider.id, amountCents: 100 },
      { from: outsider.id, to: ann.id, amountCents: 100 },
      { from: ann.id, to: bob.id, amountCents: 0 },
      { from: ann.id, to: bob.id, amountCents: -1 },
      { from: ann.id, to: bob.id, amountCents: 1.5 },
      { from: ann.id, to: bob.id },
      { to: bob.id, amountCents: 100 },
    ];
    for (const b of bads) {
      const r = await settle(ann, g, b);
      assert.equal(r.status, 400, JSON.stringify(b));
    }
  });

  test("settlements are listed oldest first and need membership", async () => {
    const g = await newGroup(ann, [bob]);
    const a = await settle(ann, g, { from: ann.id, to: bob.id, amountCents: 1 });
    const b = await settle(bob, g, { from: bob.id, to: ann.id, amountCents: 2 });
    const list = await ann.api.get(`/api/groups/${g}/settlements`);
    assert.equal(list.status, 200);
    assert.deepEqual(list.body.settlements.map((x) => x.id), [a.body.id, b.body.id]);
    assert.equal((await outsider.api.get(`/api/groups/${g}/settlements`)).status, 403);
    assert.equal((await outsider.api.post(`/api/groups/${g}/settlements`, { from: ann.id, to: bob.id, amountCents: 1 })).status, 403);
  });

  test("the feed lists events newest first with the pinned types, actors and data", async () => {
    const g = await newGroup(ann, [], "Feed");
    await ann.api.post(`/api/groups/${g}/members`, { email: bob.email });
    const e = await addExpense(bob, g, equalExpense(bob.id, 1250, [ann.id, bob.id], "Lunch"));
    const st = (await settle(ann, g, { from: ann.id, to: bob.id, amountCents: 625 })).body;
    await bob.api.del(`/api/groups/${g}/expenses/${e.id}`);
    const r = await ann.api.get(`/api/groups/${g}/activity`);
    assert.equal(r.status, 200);
    const a = r.body.activity;
    assert.deepEqual(a.map((x) => x.type), ["expense.deleted", "settlement.created", "expense.created", "member.added", "group.created"]);
    assert.deepEqual(a.map((x) => x.actorId), [bob.id, ann.id, bob.id, ann.id, ann.id]);
    assert.deepEqual(a[0].data, { expenseId: e.id, description: "Lunch", amountCents: 1250 });
    assert.deepEqual(a[1].data, { settlementId: st.id, from: ann.id, to: bob.id, amountCents: 625 });
    assert.deepEqual(a[2].data, { expenseId: e.id, description: "Lunch", amountCents: 1250 });
    assert.deepEqual(a[3].data, { userId: bob.id });
    assert.deepEqual(a[4].data, { name: "Feed" });
    for (const x of a) {
      assert.equal(typeof x.id, "string");
      assert.ok(!Number.isNaN(Date.parse(x.createdAt)));
    }
    assert.equal(new Set(a.map((x) => x.id)).size, a.length);
  });

  test("the feed only shows events of its own group", async () => {
    const g1 = await newGroup(ann, [], "One");
    const g2 = await newGroup(ann, [], "Two");
    await addExpense(ann, g2, equalExpense(ann.id, 100, [ann.id]));
    assert.deepEqual((await ann.api.get(`/api/groups/${g1}/activity`)).body.activity.map((x) => x.type), ["group.created"]);
    assert.equal((await ann.api.get(`/api/groups/${g2}/activity`)).body.activity.length, 2);
  });

  test("the limit query returns that many newest items", async () => {
    const g = await newGroup(ann, [bob]);
    for (let i = 0; i < 5; i += 1) await addExpense(ann, g, equalExpense(ann.id, 100 + i, [ann.id], `e${i}`));
    const r = await ann.api.get(`/api/groups/${g}/activity?limit=2`);
    assert.equal(r.status, 200);
    assert.deepEqual(r.body.activity.map((x) => x.data.description), ["e4", "e3"]);
    assert.equal((await ann.api.get(`/api/groups/${g}/activity`)).body.activity.length, 7);
  });

  test("the default limit is 50 and the maximum is 100", async () => {
    const g = await newGroup(ann);
    for (let i = 0; i < 60; i += 1) await addExpense(ann, g, equalExpense(ann.id, 100, [ann.id], `n${i}`));
    assert.equal((await ann.api.get(`/api/groups/${g}/activity`)).body.activity.length, 50);
    assert.equal((await ann.api.get(`/api/groups/${g}/activity?limit=100`)).body.activity.length, 61);
    assert.equal((await ann.api.get(`/api/groups/${g}/activity?limit=1`)).body.activity.length, 1);
  });

  test("an invalid limit is 400", async () => {
    const g = await newGroup(ann);
    for (const q of ["0", "101", "-1", "abc", "1.5", ""]) {
      const r = await ann.api.get(`/api/groups/${g}/activity?limit=${q}`);
      assert.equal(r.status, 400, `limit=${q}`);
      assert.equal(typeof r.body.error, "string");
    }
  });

  test("the feed needs a token and membership", async () => {
    const g = await newGroup(ann);
    assert.equal((await s.anon.get(`/api/groups/${g}/activity`)).status, 401);
    assert.equal((await outsider.api.get(`/api/groups/${g}/activity`)).status, 403);
    assert.equal((await ann.api.get(`/api/groups/nope/activity`)).status, 404);
  });
});
