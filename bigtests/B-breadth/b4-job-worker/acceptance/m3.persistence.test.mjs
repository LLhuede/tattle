import { test } from "node:test";
import assert from "node:assert/strict";
import { rmSync, readdirSync } from "node:fs";
import { startServer, client, tmpDir } from "../../../common/harness.mjs";
import { FAST, waitStatus, waitFor, enqueue } from "./_util.mjs";

async function withData(fn) {
  const dataDir = tmpDir("jobs-data-");
  try {
    await fn(dataDir);
  } finally {
    rmSync(dataDir, { recursive: true, force: true });
  }
}

test("finished jobs, results and stats survive a restart; ids are not reused", async () => {
  await withData(async (dataDir) => {
    let srv = await startServer({ env: FAST, dataDir });
    let api = client(srv.url);
    const payload = { deep: { list: [1, 2, 3], text: "héllo" } };
    const a = await enqueue(api, "echo", payload);
    const b = await enqueue(api, "sum", { numbers: [4, 5] });
    const c = await enqueue(api, "fail", { message: "persisted failure" }, 1);
    const before = [await waitStatus(api, a.id, "succeeded"), await waitStatus(api, b.id, "succeeded"), await waitStatus(api, c.id, "failed")];
    await srv.stop();

    srv = await startServer({ env: FAST, dataDir });
    api = client(srv.url);
    for (const j of before) assert.deepEqual((await api.get(`/jobs/${j.id}`)).body, j);
    assert.deepEqual((await api.get("/stats")).body, { queued: 0, running: 0, succeeded: 2, failed: 1 });
    const d = await enqueue(api, "echo", "after restart");
    assert.ok(![a.id, b.id, c.id].includes(d.id));
    const list = (await api.get("/jobs")).body;
    assert.deepEqual(list.items.map((j) => j.id).slice(0, 3), [a.id, b.id, c.id]);
    assert.equal(list.items[3].id, d.id);
    assert.equal(list.total, 4);
    await srv.stop();
  });
});

test("queued jobs and a job interrupted while running carry on after a restart", async () => {
  await withData(async (dataDir) => {
    let srv = await startServer({ env: FAST, dataDir });
    let api = client(srv.url);
    const slow = await enqueue(api, "sleep", { ms: 1000 }, 3);
    const e1 = await enqueue(api, "echo", "one");
    const e2 = await enqueue(api, "sum", { numbers: [1, 1] });
    await waitStatus(api, slow.id, "running");
    assert.equal((await api.get(`/jobs/${e1.id}`)).body.status, "queued");
    await srv.stop();

    srv = await startServer({ env: FAST, dataDir });
    api = client(srv.url);
    const afterRestart = (await api.get(`/jobs/${slow.id}`)).body;
    assert.ok(["queued", "running"].includes(afterRestart.status), `status after restart: ${afterRestart.status}`);
    const s = await waitStatus(api, slow.id, "succeeded", { timeoutMs: 15000 });
    const d1 = await waitStatus(api, e1.id, "succeeded", { timeoutMs: 15000 });
    const d2 = await waitStatus(api, e2.id, "succeeded", { timeoutMs: 15000 });
    assert.equal(s.attempts, 2, "the interrupted attempt counts");
    assert.deepEqual(s.result, { slept: 1000 });
    assert.equal(d1.attempts, 1);
    assert.equal(d1.result, "one");
    assert.equal(d2.result, 2);
    assert.ok(d1.startedAt >= s.finishedAt && d2.startedAt >= d1.finishedAt);
    await srv.stop();
  });
});

test("a running job that was on its last allowed attempt becomes failed with error 'interrupted'", async () => {
  await withData(async (dataDir) => {
    let srv = await startServer({ env: FAST, dataDir });
    let api = client(srv.url);
    const j = await enqueue(api, "sleep", { ms: 5000 }, 1);
    await waitStatus(api, j.id, "running");
    await srv.stop();

    srv = await startServer({ env: FAST, dataDir });
    api = client(srv.url);
    const g = await waitStatus(api, j.id, "failed", { timeoutMs: 4000 });
    assert.equal(g.attempts, 1);
    assert.equal(g.error, "interrupted");
    assert.equal(typeof g.finishedAt, "string");
    assert.equal((await api.get("/stats")).body.failed, 1);
    await srv.stop();
  });
});

test("a job waiting out a retry delay keeps its state and finishes after a restart", async () => {
  await withData(async (dataDir) => {
    const env = { POLL_INTERVAL_MS: "20", BACKOFF_BASE_MS: "700" };
    let srv = await startServer({ env, dataDir });
    let api = client(srv.url);
    const j = await enqueue(api, "flaky", { failTimes: 1 }, 3);
    const waiting = await waitFor("the first failure", async () => {
      const g = (await api.get(`/jobs/${j.id}`)).body;
      return g.status === "queued" && g.attempts === 1 ? g : null;
    });
    assert.equal(waiting.error, "flaky failure 1");
    await srv.stop();

    srv = await startServer({ env, dataDir });
    api = client(srv.url);
    const g = (await api.get(`/jobs/${j.id}`)).body;
    assert.equal(g.attempts, 1);
    assert.equal(g.error, "flaky failure 1");
    const done = await waitStatus(api, j.id, "succeeded", { timeoutMs: 10000 });
    assert.equal(done.attempts, 2);
    assert.deepEqual(done.result, { attempts: 2 });
    await srv.stop();
  });
});

test("a failed job can be retried after a restart", async () => {
  await withData(async (dataDir) => {
    let srv = await startServer({ env: FAST, dataDir });
    let api = client(srv.url);
    const j = await enqueue(api, "flaky", { failTimes: 1 }, 1);
    await waitStatus(api, j.id, "failed");
    await srv.stop();

    srv = await startServer({ env: FAST, dataDir });
    api = client(srv.url);
    assert.equal((await api.get(`/jobs/${j.id}`)).body.status, "failed");
    const r = await api.post(`/jobs/${j.id}/retry`);
    assert.equal(r.status, 200);
    // flaky with failTimes 1 and one attempt fails again: attempts restarted from 0
    const again = await waitFor("the retried job to finish", async () => {
      const g = (await api.get(`/jobs/${j.id}`)).body;
      return g.status === "failed" && g.attempts === 1 && g.startedAt ? g : null;
    });
    assert.equal(again.error, "flaky failure 1");
    await srv.stop();
  });
});

test("several restarts in a row keep everything consistent", async () => {
  await withData(async (dataDir) => {
    const ids = [];
    for (let round = 0; round < 3; round++) {
      const srv = await startServer({ env: FAST, dataDir });
      const api = client(srv.url);
      for (let i = 0; i < 2; i++) ids.push((await enqueue(api, "echo", `r${round}-${i}`)).id);
      await waitStatus(api, ids[ids.length - 1], "succeeded");
      const list = (await api.get("/jobs")).body;
      assert.deepEqual(list.items.map((j) => j.id), ids);
      assert.equal(new Set(ids).size, ids.length);
      await srv.stop();
    }
    const srv = await startServer({ env: FAST, dataDir });
    const api = client(srv.url);
    assert.deepEqual((await api.get("/stats")).body, { queued: 0, running: 0, succeeded: 6, failed: 0 });
    await srv.stop();
  });
});

test("the state is stored under DATA_DIR", async () => {
  const srv = await startServer({ env: FAST });
  try {
    const api = client(srv.url);
    const j = await enqueue(api, "echo", 1);
    await waitStatus(api, j.id, "succeeded");
    assert.ok(readdirSync(srv.dataDir).length > 0);
  } finally {
    await srv.stop();
  }
});
