import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { startServer, client } from "../../../common/harness.mjs";
import { FAST, waitStatus, waitFor, enqueue, sleep } from "./_util.mjs";

let srv, api;
before(async () => {
  srv = await startServer({ env: FAST }); // BACKOFF_BASE_MS=100
  api = client(srv.url);
});
after(async () => {
  await srv.stop();
});

test("a fail job with maxAttempts 1 fails at once with the default error", async () => {
  const j = await enqueue(api, "fail", null, 1);
  const done = await waitStatus(api, j.id, "failed");
  assert.equal(done.attempts, 1);
  assert.equal(done.error, "boom");
  assert.equal(done.result, null);
  assert.equal(typeof done.finishedAt, "string");
  assert.equal(typeof done.startedAt, "string");
});

test("a fail job uses its message as the error", async () => {
  const j = await enqueue(api, "fail", { message: "disk on fire" }, 1);
  const done = await waitStatus(api, j.id, "failed");
  assert.equal(done.error, "disk on fire");
  const k = await enqueue(api, "fail", { message: 42 }, 1);
  assert.equal((await waitStatus(api, k.id, "failed")).error, "boom");
});

test("a failing job is retried up to maxAttempts, with a delay between attempts", async () => {
  const t0 = Date.now();
  const j = await enqueue(api, "fail", { message: "nope" }, 3);
  const done = await waitStatus(api, j.id, "failed");
  const elapsed = Date.now() - t0;
  assert.equal(done.attempts, 3);
  assert.equal(done.error, "nope");
  assert.ok(elapsed >= 280, `expected at least 100 + 200 ms of backoff, got ${elapsed} ms`);
});

test("while it waits out a retry delay the job is queued with attempts and error set", async () => {
  const slow = await startServer({ env: { POLL_INTERVAL_MS: "20", BACKOFF_BASE_MS: "600" } });
  try {
    const c = client(slow.url);
    const j = await enqueue(c, "fail", { message: "first" }, 3);
    const waiting = await waitFor("a queued retry", async () => {
      const r = (await c.get(`/jobs/${j.id}`)).body;
      return r.status === "queued" && r.attempts === 1 ? r : null;
    });
    assert.equal(waiting.error, "first");
    assert.equal(waiting.finishedAt, null);
    await sleep(250);
    const still = (await c.get(`/jobs/${j.id}`)).body;
    assert.equal(still.status, "queued");
    assert.equal(still.attempts, 1);
  } finally {
    await slow.stop();
  }
});

test("a flaky job succeeds once enough attempts have failed", async () => {
  const t0 = Date.now();
  const j = await enqueue(api, "flaky", { failTimes: 2 }, 3);
  const done = await waitStatus(api, j.id, "succeeded");
  assert.equal(done.attempts, 3);
  assert.deepEqual(done.result, { attempts: 3 });
  assert.equal(done.error, "flaky failure 2");
  assert.ok(Date.now() - t0 >= 280);
});

test("a flaky job that needs more attempts than allowed ends up failed", async () => {
  const j = await enqueue(api, "flaky", { failTimes: 3 }, 3);
  const done = await waitStatus(api, j.id, "failed");
  assert.equal(done.attempts, 3);
  assert.equal(done.error, "flaky failure 3");
  assert.equal(done.result, null);
});

test("a flaky job with failTimes 0 succeeds at the first attempt", async () => {
  const j = await enqueue(api, "flaky", { failTimes: 0 });
  const done = await waitStatus(api, j.id, "succeeded");
  assert.equal(done.attempts, 1);
  assert.deepEqual(done.result, { attempts: 1 });
  assert.equal(done.error, null);
});

test("the retry delay doubles each time (base, 2x, 4x)", async () => {
  const slow = await startServer({ env: { POLL_INTERVAL_MS: "20", BACKOFF_BASE_MS: "200" } });
  try {
    const c = client(slow.url);
    const t0 = Date.now();
    const j = await enqueue(c, "flaky", { failTimes: 3 }, 4);
    const done = await waitStatus(c, j.id, "succeeded", { timeoutMs: 15000 });
    const elapsed = Date.now() - t0;
    assert.equal(done.attempts, 4);
    assert.ok(elapsed >= 1350, `expected at least 200 + 400 + 800 ms, got ${elapsed} ms`);
  } finally {
    await slow.stop();
  }
});

test("a job waiting for its retry does not block other jobs", async () => {
  const slow = await startServer({ env: { POLL_INTERVAL_MS: "20", BACKOFF_BASE_MS: "800" } });
  try {
    const c = client(slow.url);
    const bad = await enqueue(c, "fail", null, 2);
    const good = await enqueue(c, "echo", "hi");
    const done = await waitStatus(c, good.id, "succeeded", { timeoutMs: 3000 });
    assert.equal(done.result, "hi");
    const badNow = (await c.get(`/jobs/${bad.id}`)).body;
    assert.equal(badNow.status, "queued");
    assert.equal(badNow.attempts, 1);
    await waitStatus(c, bad.id, "failed", { timeoutMs: 8000 });
  } finally {
    await slow.stop();
  }
});

test("retry puts a failed job back in the queue with a clean slate and runs it again", async () => {
  const j = await enqueue(api, "fail", { message: "again" }, 1);
  const failed = await waitStatus(api, j.id, "failed");
  const r = await api.post(`/jobs/${j.id}/retry`);
  assert.equal(r.status, 200);
  assert.equal(r.body.id, j.id);
  assert.equal(r.body.status, "queued");
  assert.equal(r.body.attempts, 0);
  assert.equal(r.body.error, null);
  assert.equal(r.body.result, null);
  assert.equal(r.body.startedAt, null);
  assert.equal(r.body.finishedAt, null);
  const again = await waitFor("the job to fail again", async () => {
    const g = (await api.get(`/jobs/${j.id}`)).body;
    return g.status === "failed" && g.finishedAt !== failed.finishedAt ? g : null;
  });
  assert.equal(again.attempts, 1);
  assert.equal(again.error, "again");
});

test("retry on a job that is not failed is 409, and on an unknown job 404", async () => {
  const ok = await enqueue(api, "echo", 1);
  await waitStatus(api, ok.id, "succeeded");
  const r = await api.post(`/jobs/${ok.id}/retry`);
  assert.equal(r.status, 409);
  assert.equal(typeof r.body.error, "string");
  const waiting = await enqueue(api, "sleep", { ms: 500 });
  await waitStatus(api, waiting.id, "running");
  assert.equal((await api.post(`/jobs/${waiting.id}/retry`)).status, 409);
  assert.equal((await api.post("/jobs/missing/retry")).status, 404);
  await waitStatus(api, waiting.id, "succeeded");
});

test("stats and the status filter reflect failed jobs", async () => {
  const fresh = await startServer({ env: FAST });
  try {
    const c = client(fresh.url);
    const a = await enqueue(c, "fail", null, 1);
    const b = await enqueue(c, "echo", 1);
    const d = await enqueue(c, "fail", null, 1);
    await waitStatus(c, a.id, "failed");
    await waitStatus(c, b.id, "succeeded");
    await waitStatus(c, d.id, "failed");
    assert.deepEqual((await c.get("/stats")).body, { queued: 0, running: 0, succeeded: 1, failed: 2 });
    const f = await c.get("/jobs?status=failed");
    assert.deepEqual(f.body.items.map((j) => j.id), [a.id, d.id]);
    assert.equal(f.body.total, 2);
  } finally {
    await fresh.stop();
  }
});
