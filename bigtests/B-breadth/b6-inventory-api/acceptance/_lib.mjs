// Shared helpers for the Stockroom tests (not a test file).
import { startServer, client } from "../../../common/harness.mjs";

export const API_KEYS = "alice:key-admin-1:admin,bob:key-staff-2:staff";
export const ADMIN_KEY = "key-admin-1";
export const STAFF_KEY = "key-staff-2";

/** Start a server; returns { srv, anon, admin, staff, stop }. */
export async function boot({ dataDir } = {}) {
  const srv = await startServer({ env: { API_KEYS }, dataDir });
  return {
    srv,
    anon: client(srv.url),
    admin: client(srv.url, { "x-api-key": ADMIN_KEY }),
    staff: client(srv.url, { "x-api-key": STAFF_KEY }),
    stop: () => srv.stop(),
  };
}

export async function makeProduct(api, sku, extra = {}) {
  const r = await api.post("/products", { sku, name: `Product ${sku}`, ...extra });
  if (r.status !== 201) throw new Error(`could not create ${sku}: ${r.status} ${r.text}`);
  return r.body;
}

export async function move(api, sku, type, quantity, extra = {}) {
  return api.post(`/products/${sku}/movements`, { type, quantity, ...extra });
}

export function assertError(r, status) {
  if (r.status !== status) throw new Error(`expected ${status}, got ${r.status}: ${r.text}`);
  if (!r.body || typeof r.body.error !== "string" || !r.body.error) throw new Error(`expected {error: string}, got ${r.text}`);
}
