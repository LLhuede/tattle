#!/usr/bin/env node
"use strict";
const fs = require("fs");

const MONTHS = { Jan: 0, Feb: 1, Mar: 2, Apr: 3, May: 4, Jun: 5, Jul: 6, Aug: 7, Sep: 8, Oct: 9, Nov: 10, Dec: 11 };
const LINE = /^(\S+) (\S+) (\S+) \[([^\]]+)\] "([^"]*)" (\d{3}) (\d+|-)(?: "[^"]*" "[^"]*")?(?: (\d+(?:\.\d+)?))?\s*$/;
const REQ = /^([A-Za-z]+) (\S+) HTTP\/\d(?:\.\d)?$/;
const TIME = /^(\d{2})\/([A-Za-z]{3})\/(\d{4}):(\d{2}):(\d{2}):(\d{2}) ([+-])(\d{2})(\d{2})$/;

function fail(code, msg) {
  process.stderr.write(`logstat: ${msg}\n`);
  process.exit(code);
}

function parseTime(s) {
  const m = TIME.exec(s);
  if (!m || !(m[2] in MONTHS)) return null;
  const [, d, , y, hh, mm, ss, sign, oh, om] = m;
  if (+d < 1 || +d > 31 || +hh > 23 || +mm > 59 || +ss > 60) return null;
  const utc = Date.UTC(+y, MONTHS[m[2]], +d, +hh, +mm, +ss);
  return utc - (sign === "-" ? -1 : 1) * (+oh * 60 + +om) * 60000;
}

function parseLine(line) {
  const m = LINE.exec(line);
  if (!m) return null;
  const r = REQ.exec(m[5]);
  if (!r) return null;
  const t = parseTime(m[4]);
  if (t === null) return null;
  return { method: r[1], path: r[2].split("?")[0], status: m[6], time: t, rt: m[8] === undefined ? null : Number(m[8]) };
}

function parseStamp(v, name) {
  let ms;
  if (/^\d{4}-\d{2}-\d{2}$/.test(v)) ms = Date.parse(v + "T00:00:00Z");
  else if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2}(\.\d+)?)?(Z|[+-]\d{2}:\d{2})$/.test(v)) ms = Date.parse(v);
  else ms = NaN;
  if (Number.isNaN(ms)) fail(2, `invalid timestamp for ${name}: ${v}`);
  return ms;
}

function parseArgs(argv) {
  const o = { top: 10, json: false, strict: false, files: [], percentiles: [50, 90, 95, 99], since: null, until: null, status: null, method: null };
  const withValue = new Set(["--top", "--percentiles", "--since", "--until", "--status", "--method"]);
  for (let i = 0; i < argv.length; i++) {
    let a = argv[i];
    let val;
    if (a.startsWith("--") && a.includes("=")) {
      val = a.slice(a.indexOf("=") + 1);
      a = a.slice(0, a.indexOf("="));
    }
    if (a === "--help" || a === "-h") {
      process.stdout.write("Usage: logstat [options] [file ...]\n  --json --top N --percentiles LIST --since T --until T --status S --method M --strict\n");
      process.exit(0);
    } else if (a === "--json") o.json = true;
    else if (a === "--strict") o.strict = true;
    else if (withValue.has(a)) {
      if (val === undefined) {
        if (i + 1 >= argv.length) fail(2, `option ${a} needs a value`);
        val = argv[++i];
      }
      if (a === "--top") {
        if (!/^\d+$/.test(val) || Number(val) < 1) fail(2, `--top must be a positive integer: ${val}`);
        o.top = Number(val);
      } else if (a === "--percentiles") {
        const parts = val.split(",");
        if (!parts.every((p) => /^\d+$/.test(p) && +p >= 1 && +p <= 100)) fail(2, `invalid --percentiles: ${val}`);
        o.percentiles = parts.map(Number);
      } else if (a === "--since") o.since = parseStamp(val, a);
      else if (a === "--until") o.until = parseStamp(val, a);
      else if (a === "--status") {
        const parts = val.split(",");
        if (!parts.every((p) => /^[1-5]\d\d$/.test(p) || /^[1-5]xx$/.test(p))) fail(2, `invalid --status: ${val}`);
        o.status = parts;
      } else if (a === "--method") {
        if (!/^[A-Za-z]+(,[A-Za-z]+)*$/.test(val)) fail(2, `invalid --method: ${val}`);
        o.method = val.toUpperCase().split(",");
      }
    } else if (a === "-") o.files.push("-");
    else if (a.startsWith("-")) fail(2, `unknown option: ${a}`);
    else o.files.push(a);
  }
  if (o.since !== null && o.until !== null && o.since > o.until) fail(2, "--since is later than --until");
  return o;
}

function main() {
  const o = parseArgs(process.argv.slice(2));
  const sources = o.files.length ? o.files : ["-"];
  const inputs = [];
  for (const f of sources) {
    try {
      inputs.push([f === "-" ? "<stdin>" : f, fs.readFileSync(f === "-" ? 0 : f, "utf8")]);
    } catch (e) {
      fail(1, `${f}: ${e.code === "EISDIR" ? "is a directory" : e.message}`);
    }
  }
  let requests = 0;
  let malformed = 0;
  const paths = new Map();
  const status = {};
  const classes = {};
  const times = [];
  for (const [name, text] of inputs) {
    const lines = text.split("\n");
    lines.forEach((raw, idx) => {
      const line = raw.replace(/\r$/, "");
      if (line.trim() === "") return;
      const r = parseLine(line);
      if (!r) {
        if (o.strict) fail(2, `${name}:${idx + 1}: malformed log line`);
        malformed++;
        return;
      }
      if (o.since !== null && r.time < o.since) return;
      if (o.until !== null && r.time >= o.until) return;
      if (o.status && !o.status.some((s) => (s.endsWith("xx") ? r.status[0] === s[0] : r.status === s))) return;
      if (o.method && !o.method.includes(r.method.toUpperCase())) return;
      requests++;
      paths.set(r.path, (paths.get(r.path) || 0) + 1);
      status[r.status] = (status[r.status] || 0) + 1;
      const cls = r.status[0] + "xx";
      classes[cls] = (classes[cls] || 0) + 1;
      if (r.rt !== null) times.push(r.rt);
    });
  }
  const top = [...paths.entries()].sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0)).slice(0, o.top).map(([path, count]) => ({ path, count }));
  let rt = null;
  if (times.length) {
    const s = [...times].sort((a, b) => a - b);
    rt = { count: s.length, min: s[0], max: s[s.length - 1], mean: Math.round((s.reduce((a, b) => a + b, 0) / s.length) * 100) / 100 };
    for (const p of o.percentiles) rt["p" + p] = s[Math.max(1, Math.ceil((p * s.length) / 100)) - 1];
  }
  const sortedStatus = Object.fromEntries(Object.entries(status).sort((a, b) => Number(a[0]) - Number(b[0])));
  const sortedClasses = Object.fromEntries(Object.entries(classes).sort());
  if (o.json) {
    process.stdout.write(JSON.stringify({ requests, malformed, top_paths: top, status: sortedStatus, status_classes: sortedClasses, response_time_ms: rt }, null, 2) + "\n");
    return;
  }
  const out = [`requests: ${requests}`, `malformed: ${malformed}`, "top paths:"];
  for (const t of top) out.push(`  ${t.count} ${t.path}`);
  out.push("status codes:");
  for (const [c, n] of Object.entries(sortedStatus)) out.push(`  ${c} ${n}`);
  if (rt) {
    out.push("response time (ms):");
    for (const [k, v] of Object.entries(rt)) out.push(`  ${k}: ${v}`);
  } else out.push("response time (ms): n/a");
  process.stdout.write(out.join("\n") + "\n");
}

main();
