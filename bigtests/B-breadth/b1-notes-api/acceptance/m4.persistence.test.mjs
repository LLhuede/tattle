import { test } from "node:test";
import assert from "node:assert/strict";
import { startServer, client, tmpDir } from "../../../common/harness.mjs";
import { rmSync } from "node:fs";

test("notes, tags and soft-deleted notes survive a restart", async () => {
  const dataDir = tmpDir("notes-persist-");
  try {
    let srv = await startServer({ dataDir });
    let api = client(srv.url);
    const a = (await api.post("/notes", { title: "Keep", body: "body a", tags: ["Alpha", "beta"] })).body;
    const b = (await api.post("/notes", { title: "Trash", tags: ["beta"] })).body;
    await api.patch(`/notes/${a.id}`, { body: "edited" });
    await api.del(`/notes/${b.id}`);
    await srv.stop();

    srv = await startServer({ dataDir });
    api = client(srv.url);
    const g = await api.get(`/notes/${a.id}`);
    assert.equal(g.status, 200);
    assert.equal(g.body.title, "Keep");
    assert.equal(g.body.body, "edited");
    assert.deepEqual(g.body.tags, ["alpha", "beta"]);
    assert.equal(g.body.createdAt, a.createdAt);
    assert.equal((await api.get(`/notes/${b.id}`)).status, 404);
    const del = await api.get("/notes?deleted=only");
    assert.deepEqual(del.body.items.map((n) => n.id), [b.id]);
    assert.ok(del.body.items[0].deletedAt);
    assert.deepEqual((await api.get("/tags")).body.items, [{ tag: "alpha", count: 1 }, { tag: "beta", count: 1 }]);
    const restored = await api.post(`/notes/${b.id}/restore`);
    assert.equal(restored.status, 200);
    await srv.stop();
  } finally {
    rmSync(dataDir, { recursive: true, force: true });
  }
});

test("ids created after a restart never clash with earlier ones, even deleted ones", async () => {
  const dataDir = tmpDir("notes-ids-");
  try {
    let srv = await startServer({ dataDir });
    let api = client(srv.url);
    const ids = [];
    for (let i = 0; i < 3; i++) ids.push((await api.post("/notes", { title: `n${i}` })).body.id);
    await api.del(`/notes/${ids[2]}`);
    await srv.stop();

    srv = await startServer({ dataDir });
    api = client(srv.url);
    for (let i = 3; i < 6; i++) ids.push((await api.post("/notes", { title: `n${i}` })).body.id);
    assert.equal(new Set(ids).size, ids.length);
    const list = await api.get("/notes");
    assert.deepEqual(list.body.items.map((n) => n.title), ["n5", "n4", "n3", "n1", "n0"]);
    await srv.stop();
  } finally {
    rmSync(dataDir, { recursive: true, force: true });
  }
});

test("a fresh DATA_DIR starts empty", async () => {
  const srv = await startServer();
  try {
    const r = await client(srv.url).get("/notes");
    assert.equal(r.body.total, 0);
    assert.deepEqual(r.body.items, []);
  } finally {
    await srv.stop();
  }
});

test("data is written under DATA_DIR", async () => {
  const { readdirSync } = await import("node:fs");
  const srv = await startServer();
  try {
    await client(srv.url).post("/notes", { title: "on disk" });
    assert.ok(readdirSync(srv.dataDir).length > 0);
  } finally {
    await srv.stop();
  }
});

test("search, tag filters and paging work on restored data", async () => {
  const dataDir = tmpDir("notes-search-");
  try {
    let srv = await startServer({ dataDir });
    let api = client(srv.url);
    for (let i = 1; i <= 6; i++) await api.post("/notes", { title: `Item ${i}`, body: i % 2 ? "odd" : "even", tags: i % 3 ? ["x"] : ["x", "three"] });
    await srv.stop();

    srv = await startServer({ dataDir });
    api = client(srv.url);
    const odd = await api.get("/notes?q=odd");
    assert.deepEqual(odd.body.items.map((n) => n.title), ["Item 5", "Item 3", "Item 1"]);
    const three = await api.get("/notes?tag=three");
    assert.deepEqual(three.body.items.map((n) => n.title), ["Item 6", "Item 3"]);
    const page = await api.get("/notes?limit=2&offset=2");
    assert.deepEqual(page.body.items.map((n) => n.title), ["Item 4", "Item 3"]);
    assert.equal(page.body.total, 6);
    await srv.stop();
  } finally {
    rmSync(dataDir, { recursive: true, force: true });
  }
});

test("edits and deletes made after a restart persist through a second restart", async () => {
  const dataDir = tmpDir("notes-twice-");
  try {
    let srv = await startServer({ dataDir });
    let api = client(srv.url);
    const a = (await api.post("/notes", { title: "A" })).body;
    const b = (await api.post("/notes", { title: "B" })).body;
    await srv.stop();

    srv = await startServer({ dataDir });
    api = client(srv.url);
    await api.patch(`/notes/${a.id}`, { title: "A2" });
    await api.del(`/notes/${b.id}`);
    await srv.stop();

    srv = await startServer({ dataDir });
    api = client(srv.url);
    assert.equal((await api.get(`/notes/${a.id}`)).body.title, "A2");
    assert.equal((await api.get(`/notes/${b.id}`)).status, 404);
    assert.equal((await api.post(`/notes/${b.id}/restore`)).status, 200);
    await srv.stop();
  } finally {
    rmSync(dataDir, { recursive: true, force: true });
  }
});
