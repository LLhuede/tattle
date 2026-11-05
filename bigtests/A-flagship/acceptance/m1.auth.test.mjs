import { describe, test, before, after } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { client } from "../../common/harness.mjs";
import { boot, newUser } from "./_api.mjs";

function allFiles(dir) {
  const out = [];
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...allFiles(p));
    else out.push(p);
  }
  return out;
}

describe("m1 foundation and auth", () => {
  let s;
  before(async () => {
    s = await boot();
  });
  after(async () => {
    await s.stop();
  });

  test("health returns 200 with status ok", async () => {
    const r = await s.anon.get("/health");
    assert.equal(r.status, 200);
    assert.equal(r.body.status, "ok");
  });

  test("signup returns 201 with a token and the user without password fields", async () => {
    const r = await s.anon.post("/api/signup", { email: "Ann@Example.com ", name: "  Ann ", password: "longenough1" });
    assert.equal(r.status, 201);
    assert.equal(typeof r.body.token, "string");
    assert.ok(r.body.token.length > 0);
    assert.equal(r.body.user.email, "ann@example.com");
    assert.equal(r.body.user.name, "Ann");
    assert.equal(typeof r.body.user.id, "string");
    assert.deepEqual(Object.keys(r.body.user).sort(), ["email", "id", "name"]);
    assert.ok(!r.text.includes("longenough1"));
  });

  test("signup rejects invalid email, name and password with 400 and a JSON error", async () => {
    const bads = [
      { email: "nope", name: "A", password: "longenough1" },
      { email: "a@b", name: "A", password: "longenough1" },
      { email: "a b@c.co", name: "A", password: "longenough1" },
      { email: "ok1@example.com", name: "", password: "longenough1" },
      { email: "ok2@example.com", name: "   ", password: "longenough1" },
      { email: "ok3@example.com", name: "x".repeat(51), password: "longenough1" },
      { email: "ok4@example.com", name: "A", password: "short" },
      { email: "ok5@example.com", name: "A", password: "p".repeat(129) },
      { email: "ok6@example.com", name: 5, password: "longenough1" },
      { name: "A", password: "longenough1" },
    ];
    for (const b of bads) {
      const r = await s.anon.post("/api/signup", b);
      assert.equal(r.status, 400, JSON.stringify(b));
      assert.equal(typeof r.body.error, "string", JSON.stringify(b));
    }
  });

  test("signup accepts boundary lengths", async () => {
    const r = await s.anon.post("/api/signup", { email: "edge@example.com", name: "n".repeat(50), password: "12345678" });
    assert.equal(r.status, 201);
    const r2 = await s.anon.post("/api/signup", { email: "edge2@example.com", name: "n", password: "p".repeat(128) });
    assert.equal(r2.status, 201);
  });

  test("signup with an existing email (any case) returns 409", async () => {
    await newUser(s, "Dup", { email: "dup@example.com" });
    const r = await s.anon.post("/api/signup", { email: "DUP@example.com", name: "Other", password: "longenough1" });
    assert.equal(r.status, 409);
    assert.equal(typeof r.body.error, "string");
  });

  test("login works with any email case and returns a token and user", async () => {
    const u = await newUser(s, "Login", { email: "login@example.com", password: "correct-horse" });
    const r = await s.anon.post("/api/login", { email: "LOGIN@Example.com", password: "correct-horse" });
    assert.equal(r.status, 200);
    assert.equal(r.body.user.id, u.id);
    assert.equal(typeof r.body.token, "string");
    assert.notEqual(r.body.token, u.token);
  });

  test("login with wrong password or unknown email returns 401, missing fields 400", async () => {
    await newUser(s, "Wrong", { email: "wrong@example.com", password: "right-password" });
    assert.equal((await s.anon.post("/api/login", { email: "wrong@example.com", password: "nope-nope-nope" })).status, 401);
    assert.equal((await s.anon.post("/api/login", { email: "ghost@example.com", password: "right-password" })).status, 401);
    assert.equal((await s.anon.post("/api/login", { email: "wrong@example.com" })).status, 400);
    assert.equal((await s.anon.post("/api/login", { email: 7, password: "x" })).status, 400);
  });

  test("me returns the signed-in user with a signup token and a login token", async () => {
    const u = await newUser(s, "Me", { email: "me@example.com", password: "me-password-1" });
    const a = await u.api.get("/api/me");
    assert.equal(a.status, 200);
    assert.deepEqual(a.body.user, { id: u.id, email: "me@example.com", name: "Me" });
    const login = await s.anon.post("/api/login", { email: "me@example.com", password: "me-password-1" });
    const b = await client(s.url, { authorization: `Bearer ${login.body.token}` }).get("/api/me");
    assert.equal(b.status, 200);
    assert.equal(b.body.user.id, u.id);
    // the first token still works
    assert.equal((await u.api.get("/api/me")).status, 200);
  });

  test("me without a token, with a malformed header or with an unknown token returns 401", async () => {
    assert.equal((await s.anon.get("/api/me")).status, 401);
    assert.equal((await s.anon.get("/api/me", { authorization: "Bearer" })).status, 401);
    assert.equal((await s.anon.get("/api/me", { authorization: "Basic abc" })).status, 401);
    const r = await s.anon.get("/api/me", { authorization: "Bearer not-a-real-token" });
    assert.equal(r.status, 401);
    assert.equal(typeof r.body.error, "string");
  });

  test("the plain password is not stored anywhere under DATA_DIR", async () => {
    const password = "UniquePlainSecret-93717";
    await newUser(s, "Hash", { email: "hash@example.com", password });
    const login = await s.anon.post("/api/login", { email: "hash@example.com", password });
    assert.equal(login.status, 200);
    for (const f of allFiles(s.dataDir)) {
      assert.ok(!readFileSync(f).includes(password), `plain password found in ${f}`);
    }
  });
});
