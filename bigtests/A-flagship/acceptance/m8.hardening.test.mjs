import { describe, test, before, after } from "node:test";
import assert from "node:assert/strict";
import { rmSync } from "node:fs";
import { client, tmpDir } from "../../common/harness.mjs";
import { boot, newUser, newGroup, addExpense, equalExpense, balancesOf } from "./_api.mjs";

describe("m8 hardening: logout, bad input, unknown routes, persistence, concurrency", () => {
  let s, ann, bob;
  before(async () => {
    s = await boot();
    ann = await newUser(s, "Ann");
    bob = await newUser(s, "Bob");
  });
  after(async () => {
    await s.stop();
  });

  test("logout returns 204 and the token stops working", async () => {
    const u = await newUser(s, "Out1");
    assert.equal((await u.api.get("/api/me")).status, 200);
    const r = await u.api.post("/api/logout");
    assert.equal(r.status, 204);
    assert.equal(r.text, "");
    const after = await u.api.get("/api/me");
    assert.equal(after.status, 401);
    assert.equal(typeof after.body.error, "string");
  });

  test("logout only revokes the token that was used", async () => {
    const u = await newUser(s, "Out2", { email: "out2@example.com", password: "out2-password" });
    const login = await s.anon.post("/api/login", { email: "out2@example.com", password: "out2-password" });
    const second = client(s.url, { authorization: `Bearer ${login.body.token}` });
    assert.equal((await u.api.post("/api/logout")).status, 204);
    assert.equal((await second.get("/api/me")).status, 200);
    // a fresh login after logout works too
    const again = await s.anon.post("/api/login", { email: "out2@example.com", password: "out2-password" });
    assert.equal(again.status, 200);
  });

  test("logout needs a valid token and a second logout with the same token is 401", async () => {
    assert.equal((await s.anon.post("/api/logout")).status, 401);
    const u = await newUser(s, "Out3");
    assert.equal((await u.api.post("/api/logout")).status, 204);
    assert.equal((await u.api.post("/api/logout")).status, 401);
  });

  test("a revoked token cannot use group routes either", async () => {
    const u = await newUser(s, "Out4");
    const g = await newGroup(u);
    assert.equal((await u.api.get(`/api/groups/${g}`)).status, 200);
    await u.api.post("/api/logout");
    assert.equal((await u.api.get(`/api/groups/${g}`)).status, 401);
    assert.equal((await u.api.get("/api/groups")).status, 401);
  });

  test("malformed JSON or a non-object body is 400 with a JSON error", async () => {
    for (const body of ["{not json", "[1,2]", "null", '"text"', "42", "{"]) {
      const r = await s.anon.post("/api/signup", body);
      assert.equal(r.status, 400, body);
      assert.equal(typeof r.body?.error, "string", body);
    }
    const g = await ann.api.post("/api/groups", "{oops");
    assert.equal(g.status, 400);
    assert.equal(typeof g.body.error, "string");
    const h = await ann.api.post("/api/groups", "[]");
    assert.equal(h.status, 400);
  });

  test("responses are JSON with a JSON content type, errors included", async () => {
    const ok = await s.anon.get("/health");
    assert.match(ok.headers.get("content-type") ?? "", /application\/json/);
    const err = await s.anon.get("/api/me");
    assert.match(err.headers.get("content-type") ?? "", /application\/json/);
    const bad = await s.anon.post("/api/signup", "{");
    assert.match(bad.headers.get("content-type") ?? "", /application\/json/);
  });

  test("unknown routes are a JSON 404, signed in or not", async () => {
    for (const [method, path] of [["GET", "/api/nope"], ["POST", "/api/nope"], ["GET", "/definitely/not/here"], ["DELETE", "/api/groups/x/y/z"]]) {
      const anon = await s.anon.raw(method, path);
      assert.equal(anon.status, 404, `${method} ${path} anon`);
      assert.equal(typeof anon.body?.error, "string", `${method} ${path}`);
      const authed = await ann.api.raw(method, path);
      assert.equal(authed.status, 404, `${method} ${path} authed`);
    }
  });

  test("error responses carry no stack trace", async () => {
    const texts = [
      (await s.anon.post("/api/signup", "{")).text,
      (await s.anon.get("/api/nope")).text,
      (await ann.api.get("/api/groups/nope")).text,
      (await s.anon.get("/api/me")).text,
    ];
    for (const t of texts) {
      assert.doesNotMatch(t, /\n\s+at\s.*\.(js|ts|mjs|cjs)/, t);
      assert.doesNotMatch(t, /<html|<pre/i, t);
    }
  });

  test("25 concurrent expenses are all stored with distinct ids and exact balances", async () => {
    const g = await newGroup(ann, [bob]);
    const results = await Promise.all(
      Array.from({ length: 25 }, () => ann.api.post(`/api/groups/${g}/expenses`, equalExpense(ann.id, 101, [ann.id, bob.id]))),
    );
    for (const r of results) assert.equal(r.status, 201, r.text);
    assert.equal(new Set(results.map((r) => r.body.id)).size, 25);
    const list = await ann.api.get(`/api/groups/${g}/expenses`);
    assert.equal(list.body.expenses.length, 25);
    assert.deepEqual(await balancesOf(ann, g), { [ann.id]: 1250, [bob.id]: -1250 });
    const feed = await ann.api.get(`/api/groups/${g}/activity?limit=100`);
    assert.equal(feed.body.activity.filter((x) => x.type === "expense.created").length, 25);
  });

  test("concurrent signups with the same email create exactly one user", async () => {
    const results = await Promise.all(
      Array.from({ length: 8 }, () => s.anon.post("/api/signup", { email: "race@example.com", name: "Race", password: "race-password" })),
    );
    const codes = results.map((r) => r.status).sort();
    assert.deepEqual(codes, [201, 409, 409, 409, 409, 409, 409, 409]);
  });

  test("data survives a restart on the same DATA_DIR", async () => {
    const dataDir = tmpDir("bigtest-persist-");
    try {
      const first = await boot({ dataDir });
      const a = await newUser(first, "Pia", { email: "pia@example.com", password: "persist-pass-1" });
      const b = await newUser(first, "Pat", { email: "pat@example.com", password: "persist-pass-2" });
      const g = await newGroup(a, [b], "Durable");
      const e = await addExpense(a, g, equalExpense(a.id, 1001, [a.id, b.id], "Kept"));
      await a.api.post(`/api/groups/${g}/settlements`, { from: b.id, to: a.id, amountCents: 100 });
      await first.stop();

      const second = await boot({ dataDir });
      try {
        const login = await second.anon.post("/api/login", { email: "pia@example.com", password: "persist-pass-1" });
        assert.equal(login.status, 200);
        const api = client(second.url, { authorization: `Bearer ${login.body.token}` });
        const groups = (await api.get("/api/groups")).body.groups;
        assert.deepEqual(groups.map((x) => x.name), ["Durable"]);
        assert.deepEqual(groups[0].members.map((m) => m.id), [a.id, b.id]);
        const exp = (await api.get(`/api/groups/${g}/expenses/${e.id}`)).body;
        assert.equal(exp.description, "Kept");
        assert.deepEqual(exp.shares, { [a.id]: 501, [b.id]: 500 });
        const bal = (await api.get(`/api/groups/${g}/balances`)).body.balances;
        assert.deepEqual(Object.fromEntries(bal.map((x) => [x.userId, x.balanceCents])), { [a.id]: 400, [b.id]: -400 });
        const types = (await api.get(`/api/groups/${g}/activity`)).body.activity.map((x) => x.type);
        assert.deepEqual(types, ["settlement.created", "expense.created", "member.added", "group.created"]);
        assert.equal((await second.anon.post("/api/signup", { email: "pia@example.com", name: "Again", password: "persist-pass-1" })).status, 409);
      } finally {
        await second.stop();
      }
    } finally {
      rmSync(dataDir, { recursive: true, force: true });
    }
  });
});
