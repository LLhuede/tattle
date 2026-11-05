import { describe, test, before, after } from "node:test";
import assert from "node:assert/strict";
import { boot, newUser, newGroup, addExpense, equalExpense, balancesOf } from "./_api.mjs";

describe("m4 balances and settle-up", () => {
  let s, ann, bob, cy, dee, outsider;
  before(async () => {
    s = await boot();
    ann = await newUser(s, "Ann");
    bob = await newUser(s, "Bob");
    cy = await newUser(s, "Cy");
    dee = await newUser(s, "Dee");
    outsider = await newUser(s, "Out");
  });
  after(async () => {
    await s.stop();
  });

  test("a new group has a zero balance entry per member in join order", async () => {
    const g = await newGroup(ann, [bob, cy]);
    const r = await ann.api.get(`/api/groups/${g}/balances`);
    assert.equal(r.status, 200);
    assert.deepEqual(r.body.balances, [
      { userId: ann.id, balanceCents: 0 },
      { userId: bob.id, balanceCents: 0 },
      { userId: cy.id, balanceCents: 0 },
    ]);
  });

  test("the payer is credited and participants are debited", async () => {
    const g = await newGroup(ann, [bob, cy]);
    await addExpense(ann, g, equalExpense(ann.id, 3000, [ann.id, bob.id, cy.id]));
    assert.deepEqual(await balancesOf(bob, g), { [ann.id]: 2000, [bob.id]: -1000, [cy.id]: -1000 });
  });

  test("paying for two people only gives the payer a balance of half", async () => {
    const g = await newGroup(ann, [bob]);
    await addExpense(ann, g, equalExpense(ann.id, 3000, [ann.id, bob.id]));
    assert.deepEqual(await balancesOf(ann, g), { [ann.id]: 1500, [bob.id]: -1500 });
  });

  test("a payer who is not a participant is credited the whole amount", async () => {
    const g = await newGroup(ann, [bob, cy]);
    await addExpense(ann, g, equalExpense(cy.id, 1000, [ann.id, bob.id]));
    assert.deepEqual(await balancesOf(ann, g), { [ann.id]: -500, [bob.id]: -500, [cy.id]: 1000 });
  });

  test("remainder cents make balances exact and always sum to zero", async () => {
    const g = await newGroup(ann, [bob, cy]);
    await addExpense(ann, g, equalExpense(ann.id, 1000, [ann.id, bob.id, cy.id]));
    await addExpense(bob, g, equalExpense(bob.id, 100, [ann.id, bob.id, cy.id]));
    const b = await balancesOf(ann, g);
    assert.deepEqual(b, { [ann.id]: 632, [bob.id]: -266, [cy.id]: -366 });
    assert.equal(Object.values(b).reduce((x, y) => x + y, 0), 0);
  });

  test("exact splits feed balances", async () => {
    const g = await newGroup(ann, [bob, cy]);
    await addExpense(ann, g, { description: "Gift", amountCents: 1000, paidBy: bob.id, split: { type: "exact", shares: { [ann.id]: 100, [cy.id]: 900 } } });
    assert.deepEqual(await balancesOf(ann, g), { [ann.id]: -100, [bob.id]: 1000, [cy.id]: -900 });
  });

  test("deleting an expense removes its effect on balances", async () => {
    const g = await newGroup(ann, [bob]);
    const e = await addExpense(ann, g, equalExpense(ann.id, 800, [ann.id, bob.id]));
    await addExpense(bob, g, equalExpense(bob.id, 200, [ann.id, bob.id]));
    assert.equal((await ann.api.del(`/api/groups/${g}/expenses/${e.id}`)).status, 204);
    assert.deepEqual(await balancesOf(ann, g), { [ann.id]: -100, [bob.id]: 100 });
  });

  test("balances are 403 for a non-member and 404 for an unknown group", async () => {
    const g = await newGroup(ann, [bob]);
    assert.equal((await outsider.api.get(`/api/groups/${g}/balances`)).status, 403);
    assert.equal((await outsider.api.get(`/api/groups/${g}/settle-up`)).status, 403);
    assert.equal((await ann.api.get(`/api/groups/nope/balances`)).status, 404);
    assert.equal((await s.anon.get(`/api/groups/${g}/balances`)).status, 401);
  });

  test("settle-up is empty for a balanced group", async () => {
    const g = await newGroup(ann, [bob]);
    assert.deepEqual((await ann.api.get(`/api/groups/${g}/settle-up`)).body, { transfers: [] });
    await addExpense(ann, g, equalExpense(ann.id, 400, [ann.id, bob.id]));
    await addExpense(bob, g, equalExpense(bob.id, 400, [ann.id, bob.id]));
    assert.deepEqual((await ann.api.get(`/api/groups/${g}/settle-up`)).body, { transfers: [] });
  });

  test("settle-up suggests payments to the creditor, ties broken by join order", async () => {
    const g = await newGroup(ann, [bob, cy]);
    await addExpense(ann, g, equalExpense(ann.id, 3000, [ann.id, bob.id, cy.id]));
    const r = await cy.api.get(`/api/groups/${g}/settle-up`);
    assert.equal(r.status, 200);
    assert.deepEqual(r.body.transfers, [
      { from: bob.id, to: ann.id, amountCents: 1000 },
      { from: cy.id, to: ann.id, amountCents: 1000 },
    ]);
  });

  test("settle-up pairs the biggest debtor with the biggest creditor first", async () => {
    // balances: ann +500, bob -200, cy -900, dee +600
    const g = await newGroup(ann, [bob, cy, dee]);
    await addExpense(ann, g, { description: "a", amountCents: 500, paidBy: ann.id, split: { type: "exact", shares: { [ann.id]: 500 } } });
    await addExpense(ann, g, { description: "b", amountCents: 1100, paidBy: dee.id, split: { type: "exact", shares: { [bob.id]: 200, [cy.id]: 900 } } });
    await addExpense(ann, g, { description: "c", amountCents: 500, paidBy: ann.id, split: { type: "exact", shares: { [dee.id]: 500 } } });
    // ann paid 500+500=1000, owes 500 => +500 ; dee paid 1100, owes 500 => +600; bob -200; cy -900
    assert.deepEqual(await balancesOf(ann, g), { [ann.id]: 500, [bob.id]: -200, [cy.id]: -900, [dee.id]: 600 });
    const r = await ann.api.get(`/api/groups/${g}/settle-up`);
    assert.deepEqual(r.body.transfers, [
      { from: cy.id, to: dee.id, amountCents: 600 },
      { from: cy.id, to: ann.id, amountCents: 300 },
      { from: bob.id, to: ann.id, amountCents: 200 },
    ]);
  });

  test("applying the suggested transfers brings every balance to zero", async () => {
    const g = await newGroup(ann, [bob, cy, dee]);
    await addExpense(ann, g, equalExpense(ann.id, 1001, [ann.id, bob.id, cy.id, dee.id]));
    await addExpense(bob, g, equalExpense(bob.id, 777, [ann.id, bob.id, cy.id]));
    await addExpense(dee, g, equalExpense(dee.id, 5000, [bob.id, cy.id, dee.id]));
    const bal = await balancesOf(ann, g);
    const { transfers } = (await ann.api.get(`/api/groups/${g}/settle-up`)).body;
    assert.ok(transfers.length <= 3);
    for (const t of transfers) {
      assert.ok(Number.isInteger(t.amountCents) && t.amountCents > 0);
      bal[t.from] += t.amountCents;
      bal[t.to] -= t.amountCents;
    }
    assert.deepEqual(Object.values(bal), [0, 0, 0, 0]);
  });
});
