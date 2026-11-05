"use strict";
const http = require("http");
const fs = require("fs");
const path = require("path");

const DATA_DIR = process.env.DATA_DIR;
const FILE = path.join(DATA_DIR, "state.json");
const keys = new Map();
for (const part of (process.env.API_KEYS || "").split(",")) {
  const [name, key, role] = part.trim().split(":");
  if (name && key && (role === "admin" || role === "staff")) keys.set(key, { name, role });
}

let state = { products: {}, movements: [], nextMovementId: 1, audit: [], nextAuditId: 1 };
if (fs.existsSync(FILE)) state = JSON.parse(fs.readFileSync(FILE, "utf8"));

function save() {
  const tmp = FILE + ".tmp";
  fs.writeFileSync(tmp, JSON.stringify(state));
  fs.renameSync(tmp, FILE);
}

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}
const bad = (m) => new HttpError(400, m);

const isInt = (v) => Number.isInteger(v);
const STAFF_ROUTES = new Set(["GET /products", "GET /products/:sku", "GET /products/:sku/movements", "POST /products/:sku/movements", "GET /reports/low-stock"]);

function route(method, pathname) {
  const parts = pathname.split("/").filter(Boolean);
  if (pathname === "/products" && (method === "GET" || method === "POST")) return { key: `${method} /products`, params: {} };
  if (parts[0] === "products" && parts.length === 2 && ["GET", "PATCH", "DELETE"].includes(method)) return { key: `${method} /products/:sku`, params: { sku: decodeURIComponent(parts[1]) } };
  if (parts[0] === "products" && parts.length === 3 && parts[2] === "movements" && ["GET", "POST"].includes(method)) return { key: `${method} /products/:sku/movements`, params: { sku: decodeURIComponent(parts[1]) } };
  if (pathname === "/reports/low-stock" && method === "GET") return { key: "GET /reports/low-stock", params: {} };
  if (pathname === "/audit-log" && method === "GET") return { key: "GET /audit-log", params: {} };
  return null;
}

function audit(actor, action, target, details) {
  state.audit.push({ id: state.nextAuditId++, at: new Date().toISOString(), actor, action, target, details });
}

function validName(v) {
  if (typeof v !== "string" || v.trim() === "" || v.length > 100) throw bad("name must be a non-empty string of at most 100 characters");
}
function validInt(v, field) {
  if (!isInt(v) || v < 0) throw bad(`${field} must be an integer >= 0`);
}

const handlers = {
  "POST /products"({ body, user }) {
    if (typeof body.sku !== "string" || !/^[A-Za-z0-9._-]{1,32}$/.test(body.sku)) throw bad("invalid sku");
    validName(body.name);
    if (body.price_cents !== undefined) validInt(body.price_cents, "price_cents");
    if (body.reorder_level !== undefined) validInt(body.reorder_level, "reorder_level");
    if (state.products[body.sku]) throw new HttpError(409, "sku already exists");
    const now = new Date().toISOString();
    const p = { sku: body.sku, name: body.name, price_cents: body.price_cents ?? 0, reorder_level: body.reorder_level ?? 0, stock: 0, created_at: now, updated_at: now };
    state.products[p.sku] = p;
    audit(user.name, "product.create", p.sku, {});
    save();
    return [201, p];
  },
  "GET /products"({ query }) {
    const q = query.get("q");
    let list = Object.values(state.products);
    if (q) list = list.filter((p) => p.sku.toLowerCase().includes(q.toLowerCase()) || p.name.toLowerCase().includes(q.toLowerCase()));
    list.sort((a, b) => (a.sku < b.sku ? -1 : a.sku > b.sku ? 1 : 0));
    return [200, { products: list }];
  },
  "GET /products/:sku"({ params }) {
    const p = state.products[params.sku];
    if (!p) throw new HttpError(404, "product not found");
    return [200, p];
  },
  "PATCH /products/:sku"({ params, body, user }) {
    if ("sku" in body || "stock" in body) throw bad("sku and stock cannot be changed");
    const fields = ["name", "price_cents", "reorder_level"].filter((f) => f in body);
    if (!fields.length) throw bad("nothing to update");
    if ("name" in body) validName(body.name);
    if ("price_cents" in body) validInt(body.price_cents, "price_cents");
    if ("reorder_level" in body) validInt(body.reorder_level, "reorder_level");
    const p = state.products[params.sku];
    if (!p) throw new HttpError(404, "product not found");
    for (const f of fields) p[f] = body[f];
    p.updated_at = new Date().toISOString();
    audit(user.name, "product.update", p.sku, { changed: [...fields].sort() });
    save();
    return [200, p];
  },
  "DELETE /products/:sku"({ params, user }) {
    const p = state.products[params.sku];
    if (!p) throw new HttpError(404, "product not found");
    if (p.stock > 0) throw new HttpError(409, "product still has stock");
    delete state.products[p.sku];
    state.movements = state.movements.filter((m) => m.sku !== p.sku);
    audit(user.name, "product.delete", p.sku, {});
    save();
    return [204, null];
  },
  "POST /products/:sku/movements"({ params, body, user }) {
    if (body.type !== "in" && body.type !== "out") throw bad("type must be 'in' or 'out'");
    if (!isInt(body.quantity) || body.quantity < 1 || body.quantity > 1000000) throw bad("quantity must be an integer between 1 and 1000000");
    if (body.note !== undefined && body.note !== null && (typeof body.note !== "string" || body.note.length > 200)) throw bad("note must be a string of at most 200 characters");
    const p = state.products[params.sku];
    if (!p) throw new HttpError(404, "product not found");
    if (body.type === "out" && body.quantity > p.stock) throw new HttpError(409, "insufficient stock");
    p.stock += body.type === "in" ? body.quantity : -body.quantity;
    const now = new Date().toISOString();
    p.updated_at = now;
    const m = { id: state.nextMovementId++, sku: p.sku, type: body.type, quantity: body.quantity, note: body.note ?? null, actor: user.name, created_at: now, stock_after: p.stock };
    state.movements.push(m);
    audit(user.name, `stock.${body.type}`, p.sku, { quantity: body.quantity, stock_after: p.stock });
    save();
    return [201, m];
  },
  "GET /products/:sku/movements"({ params, query }) {
    const type = query.get("type");
    if (type !== null && type !== "in" && type !== "out") throw bad("type must be 'in' or 'out'");
    if (!state.products[params.sku]) throw new HttpError(404, "product not found");
    return [200, { movements: state.movements.filter((m) => m.sku === params.sku && (!type || m.type === type)) }];
  },
  "GET /reports/low-stock"() {
    const list = Object.values(state.products)
      .filter((p) => p.reorder_level > 0 && p.stock <= p.reorder_level)
      .map((p) => ({ ...p, shortfall: p.reorder_level - p.stock }));
    list.sort((a, b) => b.shortfall - a.shortfall || (a.sku < b.sku ? -1 : 1));
    return [200, { products: list }];
  },
  "GET /audit-log"({ query }) {
    let limit = 50;
    if (query.has("limit")) {
      const raw = query.get("limit");
      if (!/^\d+$/.test(raw) || +raw < 1 || +raw > 200) throw bad("limit must be an integer between 1 and 200");
      limit = +raw;
    }
    let list = [...state.audit].reverse();
    if (query.get("action")) list = list.filter((e) => e.action === query.get("action"));
    if (query.get("actor")) list = list.filter((e) => e.actor === query.get("actor"));
    return [200, { entries: list.slice(0, limit) }];
  },
};

function send(res, status, body) {
  if (body === null) {
    res.writeHead(status);
    return res.end();
  }
  const text = JSON.stringify(body);
  res.writeHead(status, { "content-type": "application/json", "content-length": Buffer.byteLength(text) });
  res.end(text);
}

const server = http.createServer((req, res) => {
  const chunks = [];
  req.on("data", (c) => chunks.push(c));
  req.on("end", () => {
    try {
      const url = new URL(req.url, "http://x");
      if (req.method === "GET" && url.pathname === "/health") return send(res, 200, { status: "ok" });
      const user = keys.get(req.headers["x-api-key"]);
      if (!user) throw new HttpError(401, "missing or invalid API key");
      const r = route(req.method, url.pathname);
      if (!r) throw new HttpError(404, "not found");
      if (user.role !== "admin" && !STAFF_ROUTES.has(r.key)) throw new HttpError(403, "forbidden");
      let body = {};
      if (req.method === "POST" || req.method === "PATCH") {
        const raw = Buffer.concat(chunks).toString("utf8");
        try {
          body = JSON.parse(raw);
        } catch {
          throw bad("body must be valid JSON");
        }
        if (body === null || typeof body !== "object" || Array.isArray(body)) throw bad("body must be a JSON object");
      }
      const [status, out] = handlers[r.key]({ params: r.params, query: url.searchParams, body, user });
      send(res, status, out);
    } catch (e) {
      if (e instanceof HttpError) return send(res, e.status, { error: e.message });
      console.error(e);
      send(res, 500, { error: "internal error" });
    }
  });
});

server.listen(Number(process.env.PORT) || 3000, "127.0.0.1");
process.on("SIGTERM", () => process.exit(0));
