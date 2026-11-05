#!/usr/bin/env node
const fs = require("fs");

class Fail extends Error {
  constructor(code, msg) {
    super(msg);
    this.code = code;
  }
}
const usageErr = (m) => new Fail(2, m);
const dataErr = (m) => new Fail(1, m);

const USAGE = `usage: csvtool <subcommand> [options] [file]
subcommands:
  select    --columns a,b
  filter    --where "col>=value" [--where ...]
  sort      --by col [--desc]
  aggregate --fn count|sum|avg|min|max [--column col] [--group-by col]
options: --format csv|json, --help
`;

function parseCsv(text) {
  const rows = [];
  let row = [];
  let field = "";
  let inQ = false;
  let quotedField = false;
  let any = false;
  const endField = () => {
    row.push(field);
    field = "";
    quotedField = false;
  };
  const endRow = () => {
    endField();
    if (!(row.length === 1 && row[0] === "" && !any)) rows.push(row);
    row = [];
    any = false;
  };
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inQ) {
      if (c === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i++;
        } else inQ = false;
      } else field += c;
    } else if (c === '"' && field === "" && !quotedField) {
      inQ = true;
      quotedField = true;
      any = true;
    } else if (c === ",") {
      any = true;
      endField();
    } else if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      endRow();
    } else {
      field += c;
      any = true;
    }
  }
  if (inQ) throw dataErr("unterminated quote in input");
  if (field !== "" || row.length || any) endRow();
  return rows;
}

function load(file) {
  let text;
  try {
    text = file && file !== "-" ? fs.readFileSync(file, "utf8") : fs.readFileSync(0, "utf8");
  } catch (e) {
    throw dataErr(`cannot read ${file}: ${e.code || e.message}`);
  }
  const rows = parseCsv(text);
  if (!rows.length) throw dataErr("input has no header row");
  const header = rows[0];
  const data = rows.slice(1);
  data.forEach((r, i) => {
    if (r.length !== header.length) throw dataErr(`row ${i + 2} has ${r.length} fields, expected ${header.length}`);
  });
  return { header, data };
}

const NUM = /^-?\d+(\.\d+)?$/;
const isNum = (s) => NUM.test(s);

function parseArgs(argv, spec) {
  const opts = {};
  const pos = [];
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "-h" || a === "--help") {
      opts.help = true;
    } else if (a === "-" || !a.startsWith("--")) {
      pos.push(a);
    } else {
      let name = a.slice(2);
      let val;
      const eq = name.indexOf("=");
      if (eq >= 0) {
        val = name.slice(eq + 1);
        name = name.slice(0, eq);
      }
      if (spec.flags.includes(name)) {
        opts[name] = true;
      } else if (spec.values.includes(name)) {
        if (val === undefined) {
          if (i + 1 >= argv.length) throw usageErr(`option --${name} needs a value`);
          val = argv[++i];
        }
        if (spec.multi && spec.multi.includes(name)) (opts[name] ||= []).push(val);
        else opts[name] = val;
      } else throw usageErr(`unknown option --${name}`);
    }
  }
  return { opts, pos };
}

function col(header, name) {
  const i = header.indexOf(name);
  if (i < 0) throw usageErr(`unknown column '${name}'`);
  return i;
}

function csvField(v) {
  v = String(v);
  return /[",\r\n]/.test(v) ? '"' + v.replace(/"/g, '""') + '"' : v;
}
function render(header, rows, format) {
  if (format === "json") {
    return JSON.stringify(rows.map((r) => Object.fromEntries(header.map((h, i) => [h, r[i]])))) + "\n";
  }
  return [header, ...rows.map((r) => r.map((v) => (v === null ? "" : v)))].map((r) => r.map(csvField).join(",")).join("\n") + "\n";
}

function run(argv) {
  if (argv[0] === "--help" || argv[0] === "-h") return [USAGE, ""];
  const sub = argv[0];
  if (!sub) throw usageErr("no subcommand given\n" + USAGE);
  const specs = {
    select: { values: ["columns", "format"], flags: [] },
    filter: { values: ["where", "format"], flags: [], multi: ["where"] },
    sort: { values: ["by", "format"], flags: ["desc"] },
    aggregate: { values: ["fn", "column", "group-by", "format"], flags: [] },
  };
  if (!specs[sub]) throw usageErr(`unknown subcommand '${sub}'`);
  const { opts, pos } = parseArgs(argv.slice(1), specs[sub]);
  if (opts.help) return [USAGE, ""];
  if (pos.length > 1) throw usageErr("too many arguments");
  const format = opts.format ?? "csv";
  if (!["csv", "json"].includes(format)) throw usageErr(`invalid --format '${format}'`);

  if (sub === "select" && opts.columns === undefined) throw usageErr("--columns is required");
  if (sub === "filter" && !opts.where) throw usageErr("--where is required");
  if (sub === "sort" && opts.by === undefined) throw usageErr("--by is required");
  if (sub === "aggregate") {
    if (!["count", "sum", "avg", "min", "max"].includes(opts.fn)) throw usageErr(`invalid or missing --fn '${opts.fn ?? ""}'`);
    if (opts.fn !== "count" && opts.column === undefined) throw usageErr("--column is required");
  }
  const { header, data } = load(pos[0]);

  if (sub === "select") {
    const idx = opts.columns.split(",").map((c) => col(header, c));
    return [render(idx.map((i) => header[i]), data.map((r) => idx.map((i) => r[i])), format), ""];
  }
  if (sub === "filter") {
    const conds = opts.where.map((w) => {
      let m = null;
      for (let i = 0; i < w.length && !m; i++) {
        const two = w.slice(i, i + 2);
        if ([">=", "<=", "!="].includes(two)) m = [i, two];
        else if ("=<>~".includes(w[i])) m = [i, w[i]];
      }
      if (!m) throw usageErr(`bad condition '${w}'`);
      return { i: col(header, w.slice(0, m[0])), op: m[1], v: w.slice(m[0] + m[1].length) };
    });
    const keep = (r) =>
      conds.every(({ i, op, v }) => {
        const c = r[i];
        if (op === "~") return c.includes(v);
        let cmp;
        if (isNum(c) && isNum(v)) cmp = Number(c) - Number(v);
        else cmp = c < v ? -1 : c > v ? 1 : 0;
        return { "=": cmp === 0, "!=": cmp !== 0, ">": cmp > 0, ">=": cmp >= 0, "<": cmp < 0, "<=": cmp <= 0 }[op];
      });
    return [render(header, data.filter(keep), format), ""];
  }
  if (sub === "sort") {
    const i = col(header, opts.by);
    const nonEmpty = data.map((r) => r[i]).filter((v) => v !== "");
    const numeric = nonEmpty.every(isNum);
    const sign = opts.desc ? -1 : 1;
    const withIdx = data.map((r, n) => ({ r, n }));
    withIdx.sort((a, b) => {
      const x = a.r[i];
      const y = b.r[i];
      if (x === "" || y === "") return x === y ? a.n - b.n : x === "" ? 1 : -1;
      const c = numeric ? Number(x) - Number(y) : x < y ? -1 : x > y ? 1 : 0;
      return c ? sign * c : a.n - b.n;
    });
    return [render(header, withIdx.map((o) => o.r), format), ""];
  }
  // aggregate
  const ci = opts.fn === "count" ? -1 : col(header, opts.column);
  const gi = opts["group-by"] === undefined ? -1 : col(header, opts["group-by"]);
  const groups = new Map();
  data.forEach((r, n) => {
    const k = gi < 0 ? "" : r[gi];
    if (!groups.has(k)) groups.set(k, []);
    groups.get(k).push({ r, n });
  });
  if (gi < 0 && !groups.size) groups.set("", []);
  const name = opts.fn === "count" ? "count" : `${opts.fn}_${opts.column}`;
  const out = [];
  for (const [k, items] of groups) {
    let val;
    if (opts.fn === "count") val = items.length;
    else {
      const nums = [];
      for (const { r, n } of items) {
        const v = r[ci];
        if (v === "") continue;
        if (!isNum(v)) throw dataErr(`column '${opts.column}' has non-numeric value '${v}' in row ${n + 2}`);
        nums.push(Number(v));
      }
      if (!nums.length) val = null;
      else if (opts.fn === "sum") val = nums.reduce((a, b) => a + b, 0);
      else if (opts.fn === "avg") val = nums.reduce((a, b) => a + b, 0) / nums.length;
      else if (opts.fn === "min") val = Math.min(...nums);
      else val = Math.max(...nums);
    }
    out.push(gi < 0 ? [val] : [k, val]);
  }
  const head = gi < 0 ? [name] : [header[gi], name];
  if (format === "json") {
    return [JSON.stringify(out.map((r) => Object.fromEntries(head.map((h, i) => [h, r[i]])))) + "\n", ""];
  }
  return [render(head, out.map((r) => r.map((v) => (v === null ? "" : String(v)))), "csv"), ""];
}

try {
  const [stdout] = run(process.argv.slice(2));
  process.stdout.write(stdout);
} catch (e) {
  if (!(e instanceof Fail)) throw e;
  process.stderr.write(`error: ${e.message}\n`);
  process.exitCode = e.code;
}
