const http = require("http");
const fs = require("fs");
const path = require("path");

const DATA_DIR = process.env.DATA_DIR || path.join(__dirname, "data");
const POLL = Number(process.env.POLL_INTERVAL_MS) || 100;
const BASE = process.env.BACKOFF_BASE_MS === undefined ? 1000 : Number(process.env.BACKOFF_BASE_MS);
fs.mkdirSync(DATA_DIR, { recursive: true });
const FILE = path.join(DATA_DIR, "jobs.json");

let state = { seq: 0, jobs: [], due: {} };
try {
  state = JSON.parse(fs.readFileSync(FILE, "utf8"));
} catch {
  /* fresh */
}
const now = () => new Date().toISOString();
function save() {
  fs.writeFileSync(FILE + ".tmp", JSON.stringify(state));
  fs.renameSync(FILE + ".tmp", FILE);
}
// recovery
for (const j of state.jobs) {
  if (j.status === "running") {
    if (j.attempts >= j.maxAttempts) {
      Object.assign(j, { status: "failed", error: "interrupted", finishedAt: now(), updatedAt: now() });
    } else {
      Object.assign(j, { status: "queued", updatedAt: now() });
      state.due[j.id] = 0;
    }
  }
}
save();

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}
const bad = (m) => new HttpError(400, m);
const isInt = (v) => Number.isInteger(v);

const handlers = {
  echo: async (job) => job.payload,
  sum: async (job) => job.payload.numbers.reduce((a, b) => a + b, 0),
  sleep: async (job) => {
    await new Promise((r) => setTimeout(r, job.payload.ms));
    return { slept: job.payload.ms };
  },
  fail: async (job) => {
    throw new Error(job.payload && typeof job.payload.message === "string" ? job.payload.message : "boom");
  },
  flaky: async (job) => {
    if (job.attempts <= job.payload.failTimes) throw new Error(`flaky failure ${job.attempts}`);
    return { attempts: job.attempts };
  },
};
const validators = {
  echo: () => {},
  fail: () => {},
  sum: (p) => {
    if (!p || !Array.isArray(p.numbers) || !p.numbers.every((n) => typeof n === "number" && Number.isFinite(n))) throw bad("sum needs payload.numbers");
  },
  sleep: (p) => {
    if (!p || !isInt(p.ms) || p.ms < 0 || p.ms > 10000) throw bad("sleep needs payload.ms in 0..10000");
  },
  flaky: (p) => {
    if (!p || !isInt(p.failTimes) || p.failTimes < 0) throw bad("flaky needs payload.failTimes >= 0");
  },
};

let busy = false;
async function tick() {
  if (busy) return;
  const t = Date.now();
  const job = state.jobs.find((j) => j.status === "queued" && (state.due[j.id] || 0) <= t);
  if (!job) return;
  busy = true;
  job.attempts++;
  Object.assign(job, { status: "running", startedAt: now(), updatedAt: now() });
  save();
  try {
    const result = await handlers[job.type](job);
    Object.assign(job, { status: "succeeded", result, finishedAt: now(), updatedAt: now() });
  } catch (e) {
    job.error = e.message;
    if (job.attempts < job.maxAttempts) {
      job.status = "queued";
      state.due[job.id] = Date.now() + BASE * 2 ** (job.attempts - 1);
    } else {
      Object.assign(job, { status: "failed", finishedAt: now() });
    }
    job.updatedAt = now();
  }
  save();
  busy = false;
  setImmediate(tick);
}
setInterval(tick, POLL);

async function handle(req) {
  const url = new URL(req.url, "http://x");
  const parts = url.pathname.split("/").filter(Boolean);
  let body;
  if (req.method === "POST") {
    const chunks = [];
    for await (const c of req) chunks.push(c);
    const text = Buffer.concat(chunks).toString("utf8");
    if (text.trim()) {
      try {
        body = JSON.parse(text);
      } catch {
        throw bad("invalid JSON");
      }
    }
  }
  const find = (id) => {
    const j = state.jobs.find((x) => x.id === id);
    if (!j) throw new HttpError(404, "job not found");
    return j;
  };
  if (req.method === "GET" && url.pathname === "/health") return [200, { ok: true }];
  if (req.method === "GET" && url.pathname === "/stats") {
    const s = { queued: 0, running: 0, succeeded: 0, failed: 0 };
    for (const j of state.jobs) s[j.status]++;
    return [200, s];
  }
  if (parts[0] === "jobs") {
    if (parts.length === 1 && req.method === "POST") {
      if (!body || typeof body !== "object" || Array.isArray(body)) throw bad("body must be an object");
      if (typeof body.type !== "string" || !handlers[body.type]) throw bad("unknown job type");
      const payload = body.payload === undefined ? null : body.payload;
      validators[body.type](payload);
      let maxAttempts = 3;
      if (body.maxAttempts !== undefined) {
        if (!isInt(body.maxAttempts) || body.maxAttempts < 1 || body.maxAttempts > 10) throw bad("maxAttempts must be 1..10");
        maxAttempts = body.maxAttempts;
      }
      const t = now();
      const job = { id: "j" + ++state.seq, type: body.type, payload, status: "queued", attempts: 0, maxAttempts, result: null, error: null, createdAt: t, updatedAt: t, startedAt: null, finishedAt: null };
      state.jobs.push(job);
      save();
      return [201, JSON.parse(JSON.stringify(job))];
    }
    if (parts.length === 1 && req.method === "GET") {
      const status = url.searchParams.get("status");
      if (status !== null && !["queued", "running", "succeeded", "failed"].includes(status)) throw bad("bad status");
      const items = state.jobs.filter((j) => status === null || j.status === status);
      return [200, { items, total: items.length }];
    }
    if (parts.length === 2 && req.method === "GET") return [200, find(parts[1])];
    if (parts.length === 3 && parts[2] === "retry" && req.method === "POST") {
      const j = find(parts[1]);
      if (j.status !== "failed") throw new HttpError(409, "job is not failed");
      Object.assign(j, { status: "queued", attempts: 0, error: null, result: null, startedAt: null, finishedAt: null, updatedAt: now() });
      state.due[j.id] = 0;
      save();
      return [200, j];
    }
  }
  throw new HttpError(404, "not found");
}

http
  .createServer(async (req, res) => {
    let status, payload;
    try {
      [status, payload] = await handle(req);
    } catch (e) {
      status = e.status || 500;
      payload = { error: e.status ? e.message : "internal error" };
    }
    res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(payload));
  })
  .listen(Number(process.env.PORT) || 3000);
