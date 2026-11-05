import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { startServer, client } from "../../../common/harness.mjs";
import { FAST, waitStatus, waitFor, enqueue } from "./_util.mjs";

let srv, api;
before(async () => {
  srv = await startServer({ env: FAST });
  api = client(srv.url);
});
after(async () => {
  await srv.stop();
});

const ISO = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})$/;

test("enqueue returns 201 with the full job shape and defaults", async () => {
  const r = await api.post("/jobs", { type: "sleep", payload: { ms: 1 } });
  assert.equal(r.status, 201);
  const j = r.body;
  assert.equal(typeof j.id, "string");
  assert.equal(j.type, "sleep");
  assert.deepEqual(j.payload, { ms: 1 });
  assert.equal(j.status, "queued");
  assert.equal(j.attempts, 0);
  assert.equal(j.maxAttempts, 3);
  assert.equal(j.result, null);
  assert.equal(j.error, null);
  assert.match(j.createdAt, ISO);
  assert.match(j.updatedAt, ISO);
  assert.equal(j.startedAt, null);
  assert.equal(j.finishedAt, null);
});

test("an echo job is processed and its result is the payload", async () => {
  const payload = { a: [1, 2, { b: "c" }], n: null, s: "text" };
  const j = await enqueue(api, "echo", payload);
  const done = await waitStatus(api, j.id, "succeeded");
  assert.deepEqual(done.result, payload);
  assert.equal(done.attempts, 1);
  assert.equal(done.error, null);
  assert.match(done.startedAt, ISO);
  assert.match(done.finishedAt, ISO);
  assert.ok(done.finishedAt >= done.startedAt);
  assert.equal(done.id, j.id);
});

test("echo with no payload gives a null payload and a null result", async () => {
  const r = await api.post("/jobs", { type: "echo" });
  assert.equal(r.status, 201);
  assert.equal(r.body.payload, null);
  const done = await waitStatus(api, r.body.id, "succeeded");
  assert.equal(done.result, null);
});

test("a sum job adds the numbers", async () => {
  const a = await enqueue(api, "sum", { numbers: [1, 2, 3.5] });
  const b = await enqueue(api, "sum", { numbers: [] });
  assert.equal((await waitStatus(api, a.id, "succeeded")).result, 6.5);
  assert.equal((await waitStatus(api, b.id, "succeeded")).result, 0);
});

test("maxAttempts can be set at enqueue time", async () => {
  const j = await enqueue(api, "echo", 1, 7);
  assert.equal(j.maxAttempts, 7);
});

test("invalid enqueue requests give 400 with an error message", async () => {
  const bad = [
    {},
    { payload: 1 },
    { type: "nope" },
    { type: 5 },
    { type: "echo", maxAttempts: 0 },
    { type: "echo", maxAttempts: 11 },
    { type: "echo", maxAttempts: 2.5 },
    { type: "echo", maxAttempts: "3" },
    [],
    "text",
  ];
  for (const body of bad) {
    const r = await api.post("/jobs", body);
    assert.equal(r.status, 400, JSON.stringify(body));
    assert.equal(typeof r.body.error, "string");
  }
  const raw = await api.raw("POST", "/jobs", "{broken", { "content-type": "application/json" });
  assert.equal(raw.status, 400);
});

test("payloads are validated per type when the job is enqueued", async () => {
  const bad = [
    { type: "sum" },
    { type: "sum", payload: { numbers: "1,2" } },
    { type: "sum", payload: { numbers: [1, "2"] } },
    { type: "sleep", payload: { ms: -1 } },
    { type: "sleep", payload: { ms: 10001 } },
    { type: "sleep", payload: { ms: 1.5 } },
    { type: "sleep" },
    { type: "flaky", payload: { failTimes: -1 } },
    { type: "flaky", payload: {} },
  ];
  for (const body of bad) {
    const r = await api.post("/jobs", body);
    assert.equal(r.status, 400, JSON.stringify(body));
  }
  const ok = await api.post("/jobs", { type: "flaky", payload: { failTimes: 0 } });
  assert.equal(ok.status, 201);
});

test("unknown job ids and routes give a JSON 404", async () => {
  const a = await api.get("/jobs/does-not-exist");
  assert.equal(a.status, 404);
  assert.equal(typeof a.body.error, "string");
  const b = await api.get("/nowhere");
  assert.equal(b.status, 404);
  assert.equal(typeof b.body.error, "string");
});

test("a sleep job is visible as running while it sleeps, then succeeds", async () => {
  const j = await enqueue(api, "sleep", { ms: 700 });
  const running = await waitStatus(api, j.id, "running", { timeoutMs: 5000 });
  assert.equal(running.attempts, 1);
  assert.match(running.startedAt, ISO);
  assert.equal(running.finishedAt, null);
  const done = await waitStatus(api, j.id, "succeeded");
  assert.deepEqual(done.result, { slept: 700 });
  assert.ok(Date.parse(done.finishedAt) - Date.parse(done.startedAt) >= 600);
});

test("one job runs at a time, in creation order", async () => {
  const first = await enqueue(api, "sleep", { ms: 400 });
  const rest = [];
  for (let i = 0; i < 3; i++) rest.push(await enqueue(api, "echo", i));
  const a = await waitStatus(api, first.id, "succeeded");
  let prev = a;
  for (const r of rest) {
    const d = await waitStatus(api, r.id, "succeeded");
    assert.ok(d.startedAt >= prev.finishedAt, `job ${d.id} started before ${prev.id} finished`);
    prev = d;
  }
});

test("GET /jobs lists oldest first with a total and filters by status", async () => {
  const fresh = await startServer({ env: FAST });
  try {
    const c = client(fresh.url);
    const ids = [];
    for (let i = 0; i < 3; i++) ids.push((await enqueue(c, "echo", i)).id);
    await waitStatus(c, ids[2], "succeeded");
    const all = await c.get("/jobs");
    assert.equal(all.status, 200);
    assert.deepEqual(all.body.items.map((j) => j.id), ids);
    assert.equal(all.body.total, 3);
    const done = await c.get("/jobs?status=succeeded");
    assert.equal(done.body.total, 3);
    const queued = await c.get("/jobs?status=queued");
    assert.deepEqual(queued.body.items, []);
    assert.equal(queued.body.total, 0);
    for (const bad of ["status=weird", "status="]) assert.equal((await c.get("/jobs?" + bad)).status, 400, bad);
  } finally {
    await fresh.stop();
  }
});

test("GET /stats counts jobs by status and always has all four keys", async () => {
  const fresh = await startServer({ env: FAST });
  try {
    const c = client(fresh.url);
    assert.deepEqual((await c.get("/stats")).body, { queued: 0, running: 0, succeeded: 0, failed: 0 });
    const a = await enqueue(c, "echo", 1);
    const b = await enqueue(c, "sum", { numbers: [1] });
    await waitStatus(c, a.id, "succeeded");
    await waitStatus(c, b.id, "succeeded");
    const s = (await c.get("/stats")).body;
    assert.deepEqual(s, { queued: 0, running: 0, succeeded: 2, failed: 0 });
    const slow = await enqueue(c, "sleep", { ms: 600 });
    await enqueue(c, "echo", 2);
    await waitStatus(c, slow.id, "running");
    const mid = (await c.get("/stats")).body;
    assert.equal(mid.running, 1);
    assert.equal(mid.queued, 1);
    assert.equal(mid.succeeded, 2);
    await waitFor("all done", async () => (await c.get("/stats")).body.succeeded === 4);
  } finally {
    await fresh.stop();
  }
});
