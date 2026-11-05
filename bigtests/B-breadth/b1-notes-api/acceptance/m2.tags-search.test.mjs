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

const mk = async (b) => (await api.post("/notes", b)).body;
const titles = (r) => r.body.items.map((n) => n.title);

test("tags are trimmed, lower-cased and de-duplicated in order", async () => {
  const r = await api.post("/notes", { title: "t", tags: [" Work ", "home", "WORK", "Home", "z"] });
  assert.equal(r.status, 201);
  assert.deepEqual(r.body.tags, ["work", "home", "z"]);
});

test("invalid tags are rejected", async () => {
  for (const tags of ["x", [1], ["ok", ""], ["  "], ["a".repeat(31)], ["a", null]]) {
    const r = await api.post("/notes", { title: "t", tags });
    assert.equal(r.status, 400, JSON.stringify(tags));
  }
  assert.equal((await api.post("/notes", { title: "t", tags: ["a".repeat(30)] })).status, 201);
});

test("at most 10 tags after de-duplication", async () => {
  const eleven = Array.from({ length: 11 }, (_, i) => `t${i}`);
  assert.equal((await api.post("/notes", { title: "t", tags: eleven })).status, 400);
  assert.equal((await api.post("/notes", { title: "t", tags: eleven.slice(0, 10) })).status, 201);
  const dup = [...eleven.slice(0, 10), "T0", "t1"];
  assert.equal((await api.post("/notes", { title: "t", tags: dup })).status, 201);
});

test("patch replaces tags with the same normalisation", async () => {
  const n = await mk({ title: "p", tags: ["a", "b"] });
  const r = await api.patch(`/notes/${n.id}`, { tags: ["B", "c", "b"] });
  assert.equal(r.status, 200);
  assert.deepEqual(r.body.tags, ["b", "c"]);
  const cleared = await api.patch(`/notes/${n.id}`, { tags: [] });
  assert.deepEqual(cleared.body.tags, []);
});

test("q matches title or body, case-insensitively", async () => {
  await mk({ title: "Grocery list", body: "milk and eggs" });
  await mk({ title: "Meeting", body: "discuss GROCERY budget" });
  await mk({ title: "Unrelated", body: "nothing here" });
  const r = await api.get("/notes?q=grocery");
  assert.equal(r.status, 200);
  assert.deepEqual(titles(r).sort(), ["Grocery list", "Meeting"]);
  const r2 = await api.get("/notes?q=MILK");
  assert.deepEqual(titles(r2), ["Grocery list"]);
  assert.equal(r2.body.total, 1);
});

test("an empty q matches everything and a q with no hits gives an empty page", async () => {
  const all = await api.get("/notes");
  const empty = await api.get("/notes?q=");
  assert.equal(empty.body.total, all.body.total);
  const none = await api.get("/notes?q=zzzqqq-no-match");
  assert.equal(none.status, 200);
  assert.deepEqual(none.body.items, []);
  assert.equal(none.body.total, 0);
});

test("q treats special characters literally", async () => {
  await mk({ title: "Price (50%) [draft]", body: "a.b*c" });
  await mk({ title: "plain", body: "axbbc" });
  assert.deepEqual(titles(await api.get("/notes?q=" + encodeURIComponent("(50%)"))), ["Price (50%) [draft]"]);
  assert.deepEqual(titles(await api.get("/notes?q=" + encodeURIComponent("a.b*c"))), ["Price (50%) [draft]"]);
});

test("tag filters by one tag, normalised", async () => {
  await mk({ title: "R1", tags: ["recipe", "dinner"] });
  await mk({ title: "R2", tags: ["recipe"] });
  await mk({ title: "W1", tags: ["work"] });
  const r = await api.get("/notes?tag=Recipe");
  assert.deepEqual(titles(r).sort(), ["R1", "R2"]);
  assert.equal(r.body.total, 2);
});

test("repeated tag parameters require all tags", async () => {
  const r = await api.get("/notes?tag=recipe&tag=dinner");
  assert.deepEqual(titles(r), ["R1"]);
  const none = await api.get("/notes?tag=recipe&tag=work");
  assert.deepEqual(none.body.items, []);
});

test("q and tag combine", async () => {
  await mk({ title: "Pasta night", body: "boil water", tags: ["recipe"] });
  const r = await api.get("/notes?q=pasta&tag=recipe");
  assert.deepEqual(titles(r), ["Pasta night"]);
  const r2 = await api.get("/notes?q=pasta&tag=work");
  assert.deepEqual(r2.body.items, []);
});

test("GET /tags counts live notes, sorted by count desc then tag asc", async () => {
  const fresh = await startServer();
  try {
    const c = client(fresh.url);
    await c.post("/notes", { title: "1", tags: ["beta", "alpha"] });
    await c.post("/notes", { title: "2", tags: ["beta", "gamma"] });
    await c.post("/notes", { title: "3", tags: ["beta", "alpha", "delta"] });
    await c.post("/notes", { title: "4", tags: [] });
    const r = await c.get("/tags");
    assert.equal(r.status, 200);
    assert.deepEqual(r.body.items, [
      { tag: "beta", count: 3 },
      { tag: "alpha", count: 2 },
      { tag: "delta", count: 1 },
      { tag: "gamma", count: 1 },
    ]);
  } finally {
    await fresh.stop();
  }
});

test("GET /tags reflects tag edits", async () => {
  const fresh = await startServer();
  try {
    const c = client(fresh.url);
    const n = (await c.post("/notes", { title: "1", tags: ["a", "b"] })).body;
    await c.patch(`/notes/${n.id}`, { tags: ["b"] });
    const r = await c.get("/tags");
    assert.deepEqual(r.body.items, [{ tag: "b", count: 1 }]);
  } finally {
    await fresh.stop();
  }
});
