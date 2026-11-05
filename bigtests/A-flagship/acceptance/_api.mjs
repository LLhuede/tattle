// Shared helpers for the hidden tests of suite A (not a test file: no .test.mjs suffix).
import assert from "node:assert/strict";
import { startServer, client } from "../../common/harness.mjs";

let counter = 0;

export async function boot(opts) {
  const s = await startServer(opts);
  return { ...s, anon: client(s.url) };
}

export async function newUser(s, name = "User", opts = {}) {
  counter += 1;
  const email = opts.email ?? `${name.toLowerCase()}${counter}.${Date.now()}@example.com`;
  const password = opts.password ?? `pw-${counter}-secret`;
  const r = await s.anon.post("/api/signup", { email, name, password });
  assert.equal(r.status, 201, `signup failed: ${r.text}`);
  return {
    id: r.body.user.id,
    email,
    name,
    password,
    token: r.body.token,
    api: client(s.url, { authorization: `Bearer ${r.body.token}` }),
  };
}

/** A group created by `owner` with the other users added by email. */
export async function newGroup(owner, others = [], name = "Trip") {
  const g = await owner.api.post("/api/groups", { name });
  assert.equal(g.status, 201, `group create failed: ${g.text}`);
  for (const u of others) {
    const r = await owner.api.post(`/api/groups/${g.body.id}/members`, { email: u.email });
    assert.equal(r.status, 201, `add member failed: ${r.text}`);
  }
  return g.body.id;
}

export function equalExpense(paidBy, amountCents, participants, description = "Dinner") {
  return { description, amountCents, paidBy, split: { type: "equal", participants } };
}

export async function addExpense(user, groupId, body) {
  const r = await user.api.post(`/api/groups/${groupId}/expenses`, body);
  assert.equal(r.status, 201, `expense create failed: ${r.text}`);
  return r.body;
}

export async function balancesOf(user, groupId) {
  const r = await user.api.get(`/api/groups/${groupId}/balances`);
  assert.equal(r.status, 200, r.text);
  return Object.fromEntries(r.body.balances.map((b) => [b.userId, b.balanceCents]));
}
