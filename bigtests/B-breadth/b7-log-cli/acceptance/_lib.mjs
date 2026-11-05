// Shared helpers for the logstat tests (not a test file).
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { runCli, tmpDir } from "../../../common/harness.mjs";

const MON = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const p2 = (n) => String(n).padStart(2, "0");

/** Apache time string. `at` is { d, mon (1-12), y, h, mi, s, tz } with defaults 10 Oct 2023 13:55:36 -0700. */
export function stamp({ d = 10, mon = 10, y = 2023, h = 13, mi = 55, s = 36, tz = "-0700" } = {}) {
  return `${p2(d)}/${MON[mon - 1]}/${y}:${p2(h)}:${p2(mi)}:${p2(s)} ${tz}`;
}

/** One log line. rt is the optional trailing response time; combined adds referer + user agent. */
export function line({ host = "127.0.0.1", ident = "-", user = "-", at = {}, method = "GET", path = "/", proto = "HTTP/1.1", status = 200, bytes = 512, rt, combined = false } = {}) {
  let s = `${host} ${ident} ${user} [${stamp(at)}] "${method} ${path} ${proto}" ${status} ${bytes}`;
  if (combined) s += ` "http://ref.example/page" "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 Chrome/118.0"`;
  if (rt !== undefined) s += ` ${rt}`;
  return s;
}

export const lines = (...ls) => ls.join("\n") + "\n";

export function run(args, input) {
  return runCli(args, { input });
}

/** Run with --json and parse stdout. Fails loudly (with stderr) if the exit code isn't 0. */
export function runJson(args, input) {
  const r = runCli(["--json", ...args], { input });
  if (r.code !== 0) throw new Error(`exit ${r.code}; stderr: ${r.stderr}`);
  return JSON.parse(r.stdout);
}

export function tempFile(name, content) {
  const dir = tmpDir("logstat-");
  mkdirSync(dir, { recursive: true });
  const p = join(dir, name);
  writeFileSync(p, content);
  return p;
}
