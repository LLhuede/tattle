import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { startServer, client } from "../../../common/harness.mjs";

let srv, api;
before(async () => {
  srv = await startServer();
  api = client(srv.url);
});
after(async () => {
  await srv.stop();
});

const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;

test("health returns 200 with ok true", async () => {
  const r = await api.get("/health");
  assert.equal(r.status, 200);
  assert.equal(r.body.ok, true);
});

test("create returns 201 with the full note shape and defaults", async () => {
  const r = await api.post("/notes", { title: "Hello" });
  assert.equal(r.status, 201);
  const n = r.body;
  assert.equal(typeof n.id, "string");
  assert.ok(n.id.length > 0);
  assert.equal(n.title, "Hello");
  assert.equal(n.body, "");
  assert.deepEqual(n.tags, []);
  assert.match(n.createdAt, ISO);
  assert.match(n.updatedAt, ISO);
  assert.equal(n.deletedAt, null);
});

test("title is trimmed and ids are unique", async () => {
  const a = await api.post("/notes", { title: "  padded  ", body: "text" });
  const b = await api.post("/notes", { title: "other" });
  assert.equal(a.body.title, "padded");
  assert.equal(a.body.body, "text");
  assert.notEqual(a.body.id, b.body.id);
});

test("create rejects a missing, non-string, blank or overlong title", async () => {
  for (const body of [{}, { title: 5 }, { title: "   " }, { title: "" }, { title: "x".repeat(201) }, { body: "no title" }]) {
    const r = await api.post("/notes", body);
    assert.equal(r.status, 400, JSON.stringify(body).slice(0, 60));
    assert.equal(typeof r.body.error, "string");
  }
  const ok = await api.post("/notes", { title: "x".repeat(200) });
  assert.equal(ok.status, 201);
});

test("create rejects a non-string or overlong body", async () => {
  assert.equal((await api.post("/notes", { title: "t", body: 12 })).status, 400);
  assert.equal((await api.post("/notes", { title: "t", body: "b".repeat(10001) })).status, 400);
  assert.equal((await api.post("/notes", { title: "t", body: "b".repeat(10000) })).status, 201);
});

test("invalid JSON in the request body gives 400 with an error message", async () => {
  const r = await api.raw("POST", "/notes", "{not json", { "content-type": "application/json" });
  assert.equal(r.status, 400);
  assert.equal(typeof r.body.error, "string");
});

test("get returns the note that was created", async () => {
  const c = await api.post("/notes", { title: "Fetch me", body: "b" });
  const r = await api.get(`/notes/${c.body.id}`);
  assert.equal(r.status, 200);
  assert.deepEqual(r.body, c.body);
});

test("unknown note ids and unknown routes give a JSON 404", async () => {
  const a = await api.get("/notes/does-not-exist");
  assert.equal(a.status, 404);
  assert.equal(typeof a.body.error, "string");
  const b = await api.get("/no/such/route");
  assert.equal(b.status, 404);
  assert.equal(typeof b.body.error, "string");
  const c = await api.patch("/notes/does-not-exist", { title: "x" });
  assert.equal(c.status, 404);
});

test("patch changes only the given fields and bumps updatedAt", async () => {
  const c = await api.post("/notes", { title: "Before", body: "keep", tags: ["a"] });
  await new Promise((r) => setTimeout(r, 15));
  const r = await api.patch(`/notes/${c.body.id}`, { title: "  After " });
  assert.equal(r.status, 200);
  assert.equal(r.body.title, "After");
  assert.equal(r.body.body, "keep");
  assert.deepEqual(r.body.tags, ["a"]);
  assert.equal(r.body.createdAt, c.body.createdAt);
  assert.ok(Date.parse(r.body.updatedAt) > Date.parse(c.body.updatedAt));
  const g = await api.get(`/notes/${c.body.id}`);
  assert.equal(g.body.title, "After");
});

test("patch with an empty object succeeds and changes nothing", async () => {
  const c = await api.post("/notes", { title: "Same", body: "b" });
  const r = await api.patch(`/notes/${c.body.id}`, {});
  assert.equal(r.status, 200);
  assert.equal(r.body.title, "Same");
  assert.equal(r.body.body, "b");
});

test("patch validates its fields and leaves the note unchanged on error", async () => {
  const c = await api.post("/notes", { title: "Stable", body: "b" });
  for (const body of [{ title: "" }, { title: 3 }, { body: false }, { tags: "nope" }]) {
    const r = await api.patch(`/notes/${c.body.id}`, body);
    assert.equal(r.status, 400, JSON.stringify(body));
  }
  const g = await api.get(`/notes/${c.body.id}`);
  assert.equal(g.body.title, "Stable");
  assert.equal(g.body.body, "b");
});
