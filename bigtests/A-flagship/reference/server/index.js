import http from "node:http";
import { readFileSync, writeFileSync, renameSync, existsSync, mkdirSync, statSync } from "node:fs";
import { join, extname, normalize, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { randomBytes, randomUUID, scryptSync, timingSafeEqual } from "node:crypto";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DATA_DIR = process.env.DATA_DIR || join(ROOT, ".data");
const PORT = Number(process.env.PORT) || 3000;
const MAX_AMOUNT = 100_000_000;
mkdirSync(DATA_DIR, { recursive: true });
const DB_FILE = join(DATA_DIR, "db.json");

// ---------- storage: one JSON file, rewritten atomically after each mutation ----------
const db = existsSync(DB_FILE)
  ? JSON.parse(readFileSync(DB_FILE, "utf8"))
  : { users: [], tokens: {}, groups: [], expenses: [], settlements: [], activity: [] };
function save() {
  const tmp = DB_FILE + ".tmp";
  writeFileSync(tmp, JSON.stringify(db));
  renameSync(tmp, DB_FILE);
}

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}
const bad = (m) => new HttpError(400, m);

// ---------- helpers ----------
const pub = (u) => ({ id: u.id, email: u.email, name: u.name });
const now = () => new Date().toISOString();
const isStr = (v) => typeof v === "string";
const isObj = (v) => v !== null && typeof v === "object" && !Array.isArray(v);
const posInt = (v) => Number.isInteger(v) && v > 0;

function hashPassword(pw) {
  const salt = randomBytes(16);
  return `${salt.toString("hex")}:${scryptSync(pw, salt, 32).toString("hex")}`;
}
function checkPassword(pw, stored) {
  const [salt, hash] = stored.split(":");
  const a = scryptSync(pw, Buffer.from(salt, "hex"), 32);
  return timingSafeEqual(a, Buffer.from(hash, "hex"));
}
function issueToken(userId) {
  const t = randomBytes(24).toString("hex");
  db.tokens[t] = userId;
  return t;
}
function logActivity(groupId, type, actorId, data) {
  db.activity.push({ id: randomUUID(), groupId, type, actorId, createdAt: now(), data });
}
function groupOut(g) {
  return { id: g.id, name: g.name, createdBy: g.createdBy, createdAt: g.createdAt, members: g.members.map((id) => pub(db.users.find((u) => u.id === id))) };
}
function expenseOut(e) {
  return { id: e.id, groupId: e.groupId, description: e.description, title: e.description, amountCents: e.amountCents, paidBy: e.paidBy, split: e.split, shares: e.shares, createdBy: e.createdBy, createdAt: e.createdAt };
}

function balances(g) {
  const bal = new Map(g.members.map((id) => [id, 0]));
  for (const e of db.expenses) {
    if (e.groupId !== g.id) continue;
    bal.set(e.paidBy, bal.get(e.paidBy) + e.amountCents);
    for (const [u, c] of Object.entries(e.shares)) bal.set(u, bal.get(u) - c);
  }
  for (const s of db.settlements) {
    if (s.groupId !== g.id) continue;
    bal.set(s.from, bal.get(s.from) + s.amountCents);
    bal.set(s.to, bal.get(s.to) - s.amountCents);
  }
  return bal;
}

function validateExpense(g, b) {
  const text = (v, name) => {
    if (!isStr(v) || v.trim().length < 1 || v.trim().length > 100) throw bad(`${name} must be 1-100 characters`);
    return v.trim();
  };
  if (b.title === undefined && b.description === undefined) throw bad("description is required");
  const d1 = b.description === undefined ? undefined : text(b.description, "description");
  const d2 = b.title === undefined ? undefined : text(b.title, "title");
  if (d1 !== undefined && d2 !== undefined && d1 !== d2) throw bad("title and description differ");
  const description = d1 ?? d2;
  if (!posInt(b.amountCents) || b.amountCents > MAX_AMOUNT) throw bad("amountCents must be a positive integer of at most 100000000");
  if (!isStr(b.paidBy) || !g.members.includes(b.paidBy)) throw bad("paidBy must be a member");
  const s = b.split;
  if (!isObj(s)) throw bad("split is required");
  let shares = {};
  let split;
  if (s.type === "equal") {
    const p = s.participants;
    if (!Array.isArray(p) || p.length === 0 || !p.every((x) => isStr(x) && g.members.includes(x)) || new Set(p).size !== p.length) {
      throw bad("participants must be distinct members");
    }
    const base = Math.floor(b.amountCents / p.length);
    const rem = b.amountCents % p.length;
    p.forEach((u, i) => (shares[u] = base + (i < rem ? 1 : 0)));
    split = { type: "equal", participants: [...p] };
  } else if (s.type === "exact") {
    if (!isObj(s.shares) || Object.keys(s.shares).length === 0) throw bad("shares must be a non-empty object");
    let sum = 0;
    for (const [u, c] of Object.entries(s.shares)) {
      if (!g.members.includes(u) || !posInt(c)) throw bad("shares must map members to positive integers");
      sum += c;
    }
    if (sum !== b.amountCents) throw bad("shares must sum to amountCents");
    shares = { ...s.shares };
    split = { type: "exact", shares: { ...s.shares } };
  } else throw bad("split.type must be equal or exact");
  return { description, amountCents: b.amountCents, paidBy: b.paidBy, split, shares };
}

// ---------- routes ----------
const routes = [];
const route = (method, pattern, auth, fn) =>
  routes.push({ method, re: new RegExp("^" + pattern.replace(/:\w+/g, "([^/]+)") + "$"), auth, fn });

route("GET", "/health", false, () => [200, { status: "ok" }]);

route("POST", "/api/signup", false, (c) => {
  const b = c.body();
  const email = isStr(b.email) ? b.email.trim().toLowerCase() : "";
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email) || email.length > 254) throw bad("invalid email");
  if (!isStr(b.name) || b.name.trim().length < 1 || b.name.trim().length > 50) throw bad("name must be 1-50 characters");
  if (!isStr(b.password) || b.password.length < 8 || b.password.length > 128) throw bad("password must be 8-128 characters");
  if (db.users.some((u) => u.email === email)) throw new HttpError(409, "email already registered");
  const user = { id: randomUUID(), email, name: b.name.trim(), password: hashPassword(b.password) };
  db.users.push(user);
  const token = issueToken(user.id);
  save();
  return [201, { token, user: pub(user) }];
});

route("POST", "/api/login", false, (c) => {
  const b = c.body();
  if (!isStr(b.email) || !isStr(b.password)) throw bad("email and password are required");
  const user = db.users.find((u) => u.email === b.email.trim().toLowerCase());
  if (!user || !checkPassword(b.password, user.password)) throw new HttpError(401, "invalid credentials");
  const token = issueToken(user.id);
  save();
  return [200, { token, user: pub(user) }];
});

route("GET", "/api/me", true, (c) => [200, { user: pub(c.user) }]);

route("POST", "/api/logout", true, (c) => {
  delete db.tokens[c.token];
  save();
  return [204, null];
});

route("POST", "/api/groups", true, (c) => {
  const b = c.body();
  if (!isStr(b.name) || b.name.trim().length < 1 || b.name.trim().length > 60) throw bad("name must be 1-60 characters");
  const g = { id: randomUUID(), name: b.name.trim(), createdBy: c.user.id, createdAt: now(), members: [c.user.id] };
  db.groups.push(g);
  logActivity(g.id, "group.created", c.user.id, { name: g.name });
  save();
  return [201, groupOut(g)];
});

route("GET", "/api/groups", true, (c) => [200, { groups: db.groups.filter((g) => g.members.includes(c.user.id)).map(groupOut) }]);

route("GET", "/api/groups/:gid", true, (c) => [200, groupOut(c.group())]);

route("POST", "/api/groups/:gid/members", true, (c) => {
  const g = c.group();
  const b = c.body();
  if (!isStr(b.email)) throw bad("email is required");
  const u = db.users.find((x) => x.email === b.email.trim().toLowerCase());
  if (!u) throw new HttpError(404, "user not found");
  if (g.members.includes(u.id)) throw new HttpError(409, "already a member");
  g.members.push(u.id);
  logActivity(g.id, "member.added", c.user.id, { userId: u.id });
  save();
  return [201, groupOut(g)];
});

route("POST", "/api/groups/:gid/expenses", true, (c) => {
  const g = c.group();
  const v = validateExpense(g, c.body());
  const e = { id: randomUUID(), groupId: g.id, ...v, createdBy: c.user.id, createdAt: now() };
  db.expenses.push(e);
  logActivity(g.id, "expense.created", c.user.id, { expenseId: e.id, description: e.description, amountCents: e.amountCents });
  save();
  return [201, expenseOut(e)];
});

route("GET", "/api/groups/:gid/expenses", true, (c) => [200, { expenses: db.expenses.filter((e) => e.groupId === c.group().id).map(expenseOut) }]);

function findExpense(c) {
  const g = c.group();
  const e = db.expenses.find((x) => x.groupId === g.id && x.id === c.params[2]);
  if (!e) throw new HttpError(404, "expense not found");
  return e;
}
route("GET", "/api/groups/:gid/expenses/:eid", true, (c) => [200, expenseOut(findExpense(c))]);
route("DELETE", "/api/groups/:gid/expenses/:eid", true, (c) => {
  const e = findExpense(c);
  if (e.createdBy !== c.user.id) throw new HttpError(403, "only the creator can delete an expense");
  db.expenses.splice(db.expenses.indexOf(e), 1);
  logActivity(e.groupId, "expense.deleted", c.user.id, { expenseId: e.id, description: e.description, amountCents: e.amountCents });
  save();
  return [204, null];
});

route("GET", "/api/groups/:gid/balances", true, (c) => {
  const g = c.group();
  const bal = balances(g);
  return [200, { balances: g.members.map((userId) => ({ userId, balanceCents: bal.get(userId) })) }];
});

route("GET", "/api/groups/:gid/settle-up", true, (c) => {
  const g = c.group();
  const bal = balances(g);
  const transfers = [];
  for (;;) {
    let payer = null;
    let receiver = null;
    for (const id of g.members) {
      if (bal.get(id) < 0 && (payer === null || bal.get(id) < bal.get(payer))) payer = id;
      if (bal.get(id) > 0 && (receiver === null || bal.get(id) > bal.get(receiver))) receiver = id;
    }
    if (payer === null || receiver === null) break;
    const amountCents = Math.min(-bal.get(payer), bal.get(receiver));
    transfers.push({ from: payer, to: receiver, amountCents });
    bal.set(payer, bal.get(payer) + amountCents);
    bal.set(receiver, bal.get(receiver) - amountCents);
  }
  return [200, { transfers }];
});

const settlementOut = ({ id, groupId, from, to, amountCents, createdBy, createdAt }) => ({ id, groupId, from, to, amountCents, createdBy, createdAt });
route("POST", "/api/groups/:gid/settlements", true, (c) => {
  const g = c.group();
  const b = c.body();
  if (!isStr(b.from) || !isStr(b.to) || !g.members.includes(b.from) || !g.members.includes(b.to) || b.from === b.to) throw bad("from and to must be different members");
  if (!posInt(b.amountCents)) throw bad("amountCents must be a positive integer");
  const s = { id: randomUUID(), groupId: g.id, from: b.from, to: b.to, amountCents: b.amountCents, createdBy: c.user.id, createdAt: now() };
  db.settlements.push(s);
  logActivity(g.id, "settlement.created", c.user.id, { settlementId: s.id, from: s.from, to: s.to, amountCents: s.amountCents });
  save();
  return [201, settlementOut(s)];
});
route("GET", "/api/groups/:gid/settlements", true, (c) => [200, { settlements: db.settlements.filter((s) => s.groupId === c.group().id).map(settlementOut) }]);

route("GET", "/api/groups/:gid/activity", true, (c) => {
  const g = c.group();
  let limit = 50;
  if (c.query.has("limit")) {
    const raw = c.query.get("limit");
    limit = /^\d+$/.test(raw) ? Number(raw) : NaN;
    if (!(limit >= 1 && limit <= 100)) throw bad("limit must be an integer from 1 to 100");
  }
  const items = db.activity.filter((a) => a.groupId === g.id).reverse().slice(0, limit);
  return [200, { activity: items.map(({ id, type, actorId, createdAt, data }) => ({ id, type, actorId, createdAt, data })) }];
});

// ---------- static UI ----------
const TYPES = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".svg": "image/svg+xml", ".json": "application/json", ".png": "image/png", ".ico": "image/x-icon" };
function serveStatic(pathname, res) {
  const rel = normalize(decodeURIComponent(pathname === "/" ? "/index.html" : pathname));
  const file = join(ROOT, "dist", rel);
  if (!file.startsWith(join(ROOT, "dist")) || !existsSync(file) || !statSync(file).isFile()) return false;
  res.writeHead(200, { "content-type": TYPES[extname(file)] || "application/octet-stream" });
  res.end(readFileSync(file));
  return true;
}

// ---------- server ----------
function send(res, status, body) {
  if (body === null) {
    res.writeHead(status);
    return res.end();
  }
  const text = JSON.stringify(body);
  res.writeHead(status, { "content-type": "application/json; charset=utf-8", "content-length": Buffer.byteLength(text) });
  res.end(text);
}

const server = http.createServer((req, res) => {
  const chunks = [];
  req.on("data", (d) => chunks.push(d));
  req.on("end", () => {
    try {
      const url = new URL(req.url, "http://x");
      let hit = null;
      for (const r of routes) {
        const m = r.method === req.method && r.re.exec(url.pathname);
        if (m) {
          hit = { r, params: m };
          break;
        }
      }
      if (!hit) {
        if (req.method === "GET" && !url.pathname.startsWith("/api/") && serveStatic(url.pathname, res)) return;
        throw new HttpError(404, "not found");
      }
      const c = { params: hit.params, query: url.searchParams, user: null, token: null };
      if (hit.r.auth) {
        const m = /^Bearer (\S+)$/.exec(req.headers.authorization || "");
        const userId = m && Object.prototype.hasOwnProperty.call(db.tokens, m[1]) ? db.tokens[m[1]] : null;
        c.user = userId && db.users.find((u) => u.id === userId);
        if (!c.user) throw new HttpError(401, "missing or invalid token");
        c.token = m[1];
      }
      c.group = () => {
        const g = db.groups.find((x) => x.id === c.params[1]);
        if (!g) throw new HttpError(404, "group not found");
        if (!g.members.includes(c.user.id)) throw new HttpError(403, "not a member of this group");
        return g;
      };
      c.body = () => {
        let v;
        try {
          v = JSON.parse(Buffer.concat(chunks).toString("utf8"));
        } catch {
          throw bad("body must be valid JSON");
        }
        if (!isObj(v)) throw bad("body must be a JSON object");
        return v;
      };
      const [status, body] = hit.r.fn(c);
      send(res, status, body);
    } catch (err) {
      if (err instanceof HttpError) return send(res, err.status, { error: err.message });
      console.error(err);
      send(res, 500, { error: "internal error" });
    }
  });
});

server.listen(PORT, () => console.log(`listening on ${PORT}`));
process.on("SIGTERM", () => process.exit(0));
