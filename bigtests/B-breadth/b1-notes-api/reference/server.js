const http = require("http");
const fs = require("fs");
const path = require("path");

const DATA_DIR = process.env.DATA_DIR || path.join(__dirname, "data");
fs.mkdirSync(DATA_DIR, { recursive: true });
const FILE = path.join(DATA_DIR, "notes.json");
let state = { nextId: 1, notes: [] };
try {
  state = JSON.parse(fs.readFileSync(FILE, "utf8"));
} catch {
  /* fresh */
}
function save() {
  fs.writeFileSync(FILE + ".tmp", JSON.stringify(state));
  fs.renameSync(FILE + ".tmp", FILE);
}

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}
const bad = (m) => new HttpError(400, m);

function validTitle(v) {
  if (typeof v !== "string") throw bad("title must be a string");
  const t = v.trim();
  if (!t) throw bad("title must not be empty");
  if (t.length > 200) throw bad("title too long");
  return t;
}
function validBody(v) {
  if (typeof v !== "string") throw bad("body must be a string");
  if (v.length > 10000) throw bad("body too long");
  return v;
}
function validTags(v) {
  if (!Array.isArray(v)) throw bad("tags must be an array");
  const out = [];
  for (const t of v) {
    if (typeof t !== "string") throw bad("tags must be strings");
    const n = t.trim().toLowerCase();
    if (!n) throw bad("tag must not be empty");
    if (n.length > 30) throw bad("tag too long");
    if (!out.includes(n)) out.push(n);
  }
  if (out.length > 10) throw bad("too many tags");
  return out;
}
const isObj = (b) => b && typeof b === "object" && !Array.isArray(b);

function find(id) {
  return state.notes.find((n) => n.id === id);
}
function live(id) {
  const n = find(id);
  if (!n || n.deletedAt) throw new HttpError(404, "note not found");
  return n;
}
function intParam(params, name, def, min, max) {
  const v = params.get(name);
  if (v === null) return def;
  if (!/^\d+$/.test(v)) throw bad(`${name} must be an integer`);
  const n = Number(v);
  if (n < min || (max !== undefined && n > max)) throw bad(`${name} out of range`);
  return n;
}

async function handle(req, res) {
  const url = new URL(req.url, "http://x");
  const parts = url.pathname.split("/").filter(Boolean);
  const m = req.method;
  let body;
  if (m === "POST" || m === "PATCH") {
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
  if (m === "GET" && url.pathname === "/health") return [200, { ok: true }];
  if (parts[0] === "tags" && parts.length === 1 && m === "GET") {
    const counts = new Map();
    for (const n of state.notes) if (!n.deletedAt) for (const t of n.tags) counts.set(t, (counts.get(t) || 0) + 1);
    const items = [...counts].map(([tag, count]) => ({ tag, count })).sort((a, b) => b.count - a.count || (a.tag < b.tag ? -1 : 1));
    return [200, { items }];
  }
  if (parts[0] === "notes") {
    if (parts.length === 1 && m === "POST") {
      if (!isObj(body)) throw bad("body must be a JSON object");
      if (body.title === undefined) throw bad("title is required");
      const now = new Date().toISOString();
      const note = {
        id: "n" + state.nextId++,
        title: validTitle(body.title),
        body: body.body === undefined ? "" : validBody(body.body),
        tags: body.tags === undefined ? [] : validTags(body.tags),
        createdAt: now,
        updatedAt: now,
        deletedAt: null,
      };
      state.notes.push(note);
      save();
      return [201, note];
    }
    if (parts.length === 1 && m === "GET") {
      const p = url.searchParams;
      const limit = intParam(p, "limit", 20, 1, 100);
      const offset = intParam(p, "offset", 0, 0);
      const deleted = p.get("deleted") ?? "exclude";
      if (!["exclude", "only", "include"].includes(deleted)) throw bad("bad deleted value");
      const q = (p.get("q") ?? "").toLowerCase();
      const tags = p.getAll("tag").map((t) => t.trim().toLowerCase());
      let items = state.notes.filter((n) => {
        if (deleted === "exclude" && n.deletedAt) return false;
        if (deleted === "only" && !n.deletedAt) return false;
        if (q && !(n.title.toLowerCase().includes(q) || n.body.toLowerCase().includes(q))) return false;
        return tags.every((t) => n.tags.includes(t));
      });
      items = items.slice().reverse();
      return [200, { items: items.slice(offset, offset + limit), total: items.length, limit, offset }];
    }
    if (parts.length === 2) {
      if (m === "GET") return [200, live(parts[1])];
      if (m === "PATCH") {
        const n = live(parts[1]);
        if (!isObj(body)) throw bad("body must be a JSON object");
        const next = {};
        if (body.title !== undefined) next.title = validTitle(body.title);
        if (body.body !== undefined) next.body = validBody(body.body);
        if (body.tags !== undefined) next.tags = validTags(body.tags);
        Object.assign(n, next, { updatedAt: new Date().toISOString() });
        save();
        return [200, n];
      }
      if (m === "DELETE") {
        const n = live(parts[1]);
        n.deletedAt = new Date().toISOString();
        save();
        return [204, null];
      }
    }
    if (parts.length === 3 && parts[2] === "restore" && m === "POST") {
      const n = find(parts[1]);
      if (!n) throw new HttpError(404, "note not found");
      if (!n.deletedAt) throw new HttpError(409, "note is not deleted");
      n.deletedAt = null;
      save();
      return [200, n];
    }
  }
  throw new HttpError(404, "not found");
}

http
  .createServer(async (req, res) => {
    let status, payload;
    try {
      [status, payload] = await handle(req, res);
    } catch (e) {
      status = e.status || 500;
      payload = { error: e.status ? e.message : "internal error" };
    }
    if (payload === null) {
      res.writeHead(status).end();
    } else {
      res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(payload));
    }
  })
  .listen(Number(process.env.PORT) || 3000);
