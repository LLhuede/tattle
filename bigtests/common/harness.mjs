// Helpers for hidden acceptance tests (node:test, no dependencies).
// The app under test is in process.env.APP_DIR (a built copy of the agent's
// final code). See bigtests/CONTRACT.md.

import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, existsSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createServer } from "node:net";
import { pathToFileURL } from "node:url";

export const APP_DIR = process.env.APP_DIR ? resolve(process.env.APP_DIR) : process.cwd();

export function pkg() {
  return JSON.parse(readFileSync(join(APP_DIR, "package.json"), "utf8"));
}

export function tmpDir(prefix = "bigtest-") {
  return mkdtempSync(join(tmpdir(), prefix));
}

export function appPath(...parts) {
  return join(APP_DIR, ...parts);
}

export function exists(...parts) {
  return existsSync(join(APP_DIR, ...parts));
}

async function freePort() {
  return new Promise((ok, fail) => {
    const s = createServer();
    s.unref();
    s.on("error", fail);
    s.listen(0, () => {
      const { port } = s.address();
      s.close(() => ok(port));
    });
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/**
 * Start the app with `npm start` on a free port and a fresh DATA_DIR, and
 * wait for GET /health to return 200. Returns { url, dataDir, logs(), stop() }.
 */
export async function startServer({ env = {}, timeoutMs = 30000, dataDir } = {}) {
  const port = await freePort();
  const data = dataDir ?? tmpDir("bigtest-data-");
  let out = "";
  const child = spawn("npm", ["start", "--silent"], {
    cwd: APP_DIR,
    env: { ...process.env, ...env, PORT: String(port), DATA_DIR: data, NODE_ENV: "test" },
    stdio: ["ignore", "pipe", "pipe"],
    detached: true,
  });
  child.stdout.on("data", (d) => (out += d));
  child.stderr.on("data", (d) => (out += d));
  const url = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + timeoutMs;
  let ready = false;
  while (Date.now() < deadline && child.exitCode === null) {
    try {
      const r = await fetch(`${url}/health`);
      if (r.status === 200) {
        ready = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await sleep(200);
  }
  const stop = async () => {
    try {
      process.kill(-child.pid, "SIGTERM");
    } catch {
      /* already gone */
    }
    await sleep(100);
    if (!dataDir) rmSync(data, { recursive: true, force: true });
  };
  if (!ready) {
    await stop();
    throw new Error(`server didn't answer GET /health within ${timeoutMs} ms (exit ${child.exitCode}). Output:\n${out.slice(-2000)}`);
  }
  return { url, dataDir: data, logs: () => out, stop };
}

/** JSON HTTP client for a started server: { status, body, headers }. */
export function client(url, defaultHeaders = {}) {
  const call = async (method, path, body, headers = {}) => {
    const r = await fetch(url + path, {
      method,
      redirect: "manual",
      headers: { ...(body !== undefined ? { "content-type": "application/json" } : {}), ...defaultHeaders, ...headers },
      body: body === undefined ? undefined : typeof body === "string" ? body : JSON.stringify(body),
    });
    const text = await r.text();
    let parsed = text;
    try {
      parsed = text ? JSON.parse(text) : null;
    } catch {
      /* not JSON */
    }
    return { status: r.status, body: parsed, text, headers: r.headers };
  };
  return {
    get: (p, h) => call("GET", p, undefined, h),
    post: (p, b, h) => call("POST", p, b, h),
    put: (p, b, h) => call("PUT", p, b, h),
    patch: (p, b, h) => call("PATCH", p, b, h),
    del: (p, h) => call("DELETE", p, undefined, h),
    raw: call,
  };
}

/** The CLI entry from package.json `bin` (first one, or the named one). */
export function binPath(name) {
  const b = pkg().bin;
  if (!b) throw new Error("package.json has no bin entry");
  const rel = typeof b === "string" ? b : name ? b[name] : Object.values(b)[0];
  return join(APP_DIR, rel);
}

/** Run the CLI: { code, stdout, stderr }. */
export function runCli(args = [], { input, env = {}, name, timeoutMs = 20000 } = {}) {
  const r = spawnSync(process.execPath, [binPath(name), ...args], {
    cwd: APP_DIR,
    input,
    env: { ...process.env, ...env },
    encoding: "utf8",
    timeout: timeoutMs,
  });
  return { code: r.status, stdout: r.stdout ?? "", stderr: r.stderr ?? "" };
}

/** Import a module of the app: a path relative to APP_DIR, or the package entry if omitted. */
export async function importApp(rel) {
  let target = rel;
  if (!target) {
    const p = pkg();
    const e = p.exports;
    target = (typeof e === "string" ? e : e?.["."]?.import ?? e?.["."]?.default ?? e?.["."]?.require ?? (typeof e?.["."] === "string" ? e["."] : undefined)) ?? p.main ?? "index.js";
  }
  const mod = await import(pathToFileURL(join(APP_DIR, target)).href);
  return mod.default && Object.keys(mod).length === 1 ? { ...mod.default, default: mod.default } : mod;
}
