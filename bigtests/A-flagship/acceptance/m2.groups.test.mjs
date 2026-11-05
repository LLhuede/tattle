import { describe, test, before, after } from "node:test";
import assert from "node:assert/strict";
import { boot, newUser, newGroup } from "./_api.mjs";

describe("m2 groups and membership", () => {
  let s, ann, bob, cy;
  before(async () => {
    s = await boot();
    ann = await newUser(s, "Ann");
    bob = await newUser(s, "Bob");
    cy = await newUser(s, "Cy");
  });
  after(async () => {
    await s.stop();
  });

  test("creating a group returns 201 with the creator as the only member", async () => {
    const r = await ann.api.post("/api/groups", { name: "  Flat  " });
    assert.equal(r.status, 201);
    assert.equal(typeof r.body.id, "string");
    assert.equal(r.body.name, "Flat");
    assert.equal(r.body.createdBy, ann.id);
    assert.ok(!Number.isNaN(Date.parse(r.body.createdAt)));
    assert.deepEqual(r.body.members, [{ id: ann.id, email: ann.email, name: "Ann" }]);
  });

  test("group creation validates the name", async () => {
    for (const name of ["", "   ", "x".repeat(61), 12, undefined]) {
      const r = await ann.api.post("/api/groups", { name });
      assert.equal(r.status, 400, JSON.stringify(name));
      assert.equal(typeof r.body.error, "string");
    }
    assert.equal((await ann.api.post("/api/groups", { name: "x".repeat(60) })).status, 201);
  });

  test("group routes need a token", async () => {
    assert.equal((await s.anon.post("/api/groups", { name: "A" })).status, 401);
    assert.equal((await s.anon.get("/api/groups")).status, 401);
    const id = await newGroup(ann);
    assert.equal((await s.anon.get(`/api/groups/${id}`)).status, 401);
  });

  test("adding a member by email (any case) returns the updated group in join order", async () => {
    const id = await newGroup(ann);
    const r = await ann.api.post(`/api/groups/${id}/members`, { email: bob.email.toUpperCase() });
    assert.equal(r.status, 201);
    assert.deepEqual(r.body.members.map((m) => m.id), [ann.id, bob.id]);
    const r2 = await bob.api.post(`/api/groups/${id}/members`, { email: cy.email });
    assert.equal(r2.status, 201, "any member can add");
    assert.deepEqual(r2.body.members.map((m) => m.id), [ann.id, bob.id, cy.id]);
  });

  test("adding an unknown email is 404, a duplicate member is 409, a bad body is 400", async () => {
    const id = await newGroup(ann, [bob]);
    assert.equal((await ann.api.post(`/api/groups/${id}/members`, { email: "nobody@example.com" })).status, 404);
    assert.equal((await ann.api.post(`/api/groups/${id}/members`, { email: bob.email })).status, 409);
    assert.equal((await ann.api.post(`/api/groups/${id}/members`, {})).status, 400);
    assert.equal((await ann.api.post(`/api/groups/${id}/members`, { email: 5 })).status, 400);
  });

  test("a non-member gets 403 on reading a group and on adding members", async () => {
    const id = await newGroup(ann, [bob]);
    const g = await cy.api.get(`/api/groups/${id}`);
    assert.equal(g.status, 403);
    assert.equal(typeof g.body.error, "string");
    assert.equal((await cy.api.post(`/api/groups/${id}/members`, { email: cy.email })).status, 403);
  });

  test("an unknown group is 404 for any signed-in user", async () => {
    assert.equal((await ann.api.get("/api/groups/does-not-exist")).status, 404);
    assert.equal((await ann.api.post("/api/groups/does-not-exist/members", { email: bob.email })).status, 404);
  });

  test("listing groups returns only the caller's groups in creation order", async () => {
    const dee = await newUser(s, "Dee");
    const g1 = await newGroup(dee, [], "First");
    const g2 = await newGroup(ann, [dee], "Second");
    await newGroup(ann, [], "Not for Dee");
    const g3 = await newGroup(dee, [], "Third");
    const r = await dee.api.get("/api/groups");
    assert.equal(r.status, 200);
    assert.deepEqual(r.body.groups.map((g) => g.id), [g1, g2, g3]);
    assert.deepEqual(r.body.groups[1].members.map((m) => m.name), ["Ann", "Dee"]);
  });

  test("get group returns the group with its members for a member", async () => {
    const id = await newGroup(ann, [bob], "Ski");
    const r = await bob.api.get(`/api/groups/${id}`);
    assert.equal(r.status, 200);
    assert.equal(r.body.id, id);
    assert.equal(r.body.name, "Ski");
    assert.equal(r.body.members.length, 2);
  });

  test("a newly added member sees the group in their list", async () => {
    const eve = await newUser(s, "Eve");
    assert.deepEqual((await eve.api.get("/api/groups")).body, { groups: [] });
    const id = await newGroup(ann, [eve]);
    assert.deepEqual((await eve.api.get("/api/groups")).body.groups.map((g) => g.id), [id]);
  });
});
