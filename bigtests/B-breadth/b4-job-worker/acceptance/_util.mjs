export const FAST = { POLL_INTERVAL_MS: "20", BACKOFF_BASE_MS: "100" };

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** Poll fn until it returns a truthy value; throws with `what` on timeout. */
export async function waitFor(what, fn, { timeoutMs = 10000, everyMs = 15 } = {}) {
  const end = Date.now() + timeoutMs;
  let last;
  while (Date.now() < end) {
    last = await fn();
    if (last) return last;
    await sleep(everyMs);
  }
  throw new Error(`timed out waiting for ${what}`);
}

/** Wait until the job is in one of the statuses; returns the job body. */
export async function waitStatus(api, id, statuses, opts) {
  const want = Array.isArray(statuses) ? statuses : [statuses];
  return waitFor(`job ${id} to be ${want.join("/")}`, async () => {
    const r = await api.get(`/jobs/${id}`);
    return want.includes(r.body.status) ? r.body : null;
  }, opts);
}

export async function enqueue(api, type, payload, maxAttempts) {
  const body = { type, payload };
  if (maxAttempts !== undefined) body.maxAttempts = maxAttempts;
  const r = await api.post("/jobs", body);
  if (r.status !== 201) throw new Error(`enqueue failed: ${r.status} ${r.text}`);
  return r.body;
}
