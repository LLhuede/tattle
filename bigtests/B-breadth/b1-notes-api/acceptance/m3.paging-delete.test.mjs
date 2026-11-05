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

const mk = async (title, extra = {}) => (await api.post("/notes", { title, ...extra })).body;
const titles = (r) => r.body.items.map((n) => n.title);

test("list is newest first and reports total, limit and offset", async () => {
  for (let i = 1; i <= 5; i++) await mk(`n${i}`);
  const r = await api.get("/notes");
  assert.equal(r.status, 200);
  assert.deepEqual(titles(r), ["n5", "n4", "n3", "n2", "n1"]);
  assert.equal(r.body.total, 5);
  assert.equal(r.body.limit, 20);
  assert.equal(r.body.offset, 0);
});

test("limit and offset page through the list", async () => {
  const p1 = await api.get("/notes?limit=2");
  assert.deepEqual(titles(p1), ["n5", "n4"]);
  assert.equal(p1.body.total, 5);
  assert.equal(p1.body.limit, 2);
  const p2 = await api.get("/notes?limit=2&offset=2");
  assert.deepEqual(titles(p2), ["n3", "n2"]);
  assert.equal(p2.body.offset, 2);
  const p3 = await api.get("/notes?limit=2&offset=4");
  assert.deepEqual(titles(p3), ["n1"]);
  const beyond = await api.get("/notes?limit=2&offset=50");
  assert.equal(beyond.status, 200);
  assert.deepEqual(beyond.body.items, []);
  assert.equal(beyond.body.total, 5);
});

test("updating a note does not change list order", async () => {
  const all = await api.get("/notes");
  const oldest = all.body.items[all.body.items.length - 1];
  await api.patch(`/notes/${oldest.id}`, { body: "touched" });
  const r = await api.get("/notes");
  assert.equal(titles(r)[0], "n5");
  assert.equal(titles(r)[4], "n1");
});

test("invalid limit, offset or deleted values give 400", async () => {
  for (const qs of ["limit=0", "limit=101", "limit=abc", "limit=2.5", "limit=-1", "offset=-1", "offset=x", "deleted=maybe"]) {
    const r = await api.get("/notes?" + qs);
    assert.equal(r.status, 400, qs);
    assert.equal(typeof r.body.error, "string");
  }
  assert.equal((await api.get("/notes?limit=100")).status, 200);
  assert.equal((await api.get("/notes?limit=1")).status, 200);
});

test("total counts matches before paging when filters are used", async () => {
  for (let i = 0; i < 4; i++) await mk(`tagged${i}`, { tags: ["pg"] });
  const r = await api.get("/notes?tag=pg&limit=3");
  assert.equal(r.body.items.length, 3);
  assert.equal(r.body.total, 4);
  const r2 = await api.get("/notes?tag=pg&limit=3&offset=3");
  assert.equal(r2.body.items.length, 1);
});

test("delete returns 204 with no body and hides the note", async () => {
  const n = await mk("to delete");
  const d = await api.del(`/notes/${n.id}`);
  assert.equal(d.status, 204);
  assert.equal(d.text, "");
  assert.equal((await api.get(`/notes/${n.id}`)).status, 404);
  assert.ok(!titles(await api.get("/notes?limit=100")).includes("to delete"));
});

test("deleting twice or deleting an unknown id gives 404; patching a deleted note gives 404", async () => {
  const n = await mk("twice");
  await api.del(`/notes/${n.id}`);
  assert.equal((await api.del(`/notes/${n.id}`)).status, 404);
  assert.equal((await api.patch(`/notes/${n.id}`, { title: "x" })).status, 404);
  assert.equal((await api.del("/notes/never-existed")).status, 404);
});

test("deleted=only and deleted=include list soft-deleted notes", async () => {
  const fresh = await startServer();
  try {
    const c = client(fresh.url);
    const a = (await c.post("/notes", { title: "A" })).body;
    const b = (await c.post("/notes", { title: "B" })).body;
    await c.post("/notes", { title: "C" });
    await c.del(`/notes/${b.id}`);
    const only = await c.get("/notes?deleted=only");
    assert.deepEqual(only.body.items.map((n) => n.title), ["B"]);
    assert.equal(only.body.total, 1);
    assert.ok(only.body.items[0].deletedAt);
    const inc = await c.get("/notes?deleted=include");
    assert.deepEqual(inc.body.items.map((n) => n.title), ["C", "B", "A"]);
    assert.equal(inc.body.total, 3);
    const exc = await c.get("/notes?deleted=exclude");
    assert.deepEqual(exc.body.items.map((n) => n.title), ["C", "A"]);
    assert.equal(a.deletedAt, null);
  } finally {
    await fresh.stop();
  }
});

test("restore brings a note back; restoring a live note is 409; unknown is 404", async () => {
  const n = await mk("restore me", { tags: ["rest"] });
  await api.del(`/notes/${n.id}`);
  const r = await api.post(`/notes/${n.id}/restore`);
  assert.equal(r.status, 200);
  assert.equal(r.body.id, n.id);
  assert.equal(r.body.deletedAt, null);
  assert.equal((await api.get(`/notes/${n.id}`)).status, 200);
  assert.equal((await api.post(`/notes/${n.id}/restore`)).status, 409);
  assert.equal((await api.post("/notes/nope/restore")).status, 404);
});

test("soft-deleted notes do not count in /tags", async () => {
  const fresh = await startServer();
  try {
    const c = client(fresh.url);
    const a = (await c.post("/notes", { title: "A", tags: ["x", "y"] })).body;
    await c.post("/notes", { title: "B", tags: ["x"] });
    await c.del(`/notes/${a.id}`);
    assert.deepEqual((await c.get("/tags")).body.items, [{ tag: "x", count: 1 }]);
    await c.post(`/notes/${a.id}/restore`);
    assert.deepEqual((await c.get("/tags")).body.items, [{ tag: "x", count: 2 }, { tag: "y", count: 1 }]);
  } finally {
    await fresh.stop();
  }
});

test("search and tag filters skip soft-deleted notes unless deleted=include", async () => {
  const fresh = await startServer();
  try {
    const c = client(fresh.url);
    const a = (await c.post("/notes", { title: "findme", tags: ["k"] })).body;
    await c.del(`/notes/${a.id}`);
    assert.deepEqual((await c.get("/notes?q=findme")).body.items, []);
    assert.deepEqual((await c.get("/notes?tag=k")).body.items, []);
    assert.equal((await c.get("/notes?q=findme&deleted=include")).body.total, 1);
    assert.equal((await c.get("/notes?tag=k&deleted=only")).body.total, 1);
  } finally {
    await fresh.stop();
  }
});
