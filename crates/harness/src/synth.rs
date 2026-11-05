//! Synthetic recordings: small, realistic Claude Code sessions on three
//! TypeScript apps, recorded by driving the real recorder
//! (`capture::hook::record`) and exported with the real exporter. This keeps
//! them in exactly the same format as real recordings.
//!
//! They are clean: the "agent" does the task correctly, runs the tests and
//! they pass. Mutations then inject one defect each.
//!
//! Event shapes follow real Claude Code recordings (2.1.307): Bash success
//! has no exit code, failures arrive as `PostToolUseFailure`, every tool
//! call has a `tool_use_id`, and `Stop` carries `last_assistant_message`.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use capture::export::{ExportOptions, export};
use capture::hook::{HookEnv, record};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub enum Step {
    Read(&'static str),
    /// Replace an existing file's contents.
    Edit(&'static str, &'static str),
    /// Create a new file.
    Write(&'static str, &'static str),
    Bash {
        command: &'static str,
        stdout: &'static str,
        exit_code: i32,
        /// Files the command creates or changes (e.g. scaffolding,
        /// installs, formatters), written between Pre and Post.
        writes: &'static [(&'static str, &'static str)],
    },
}

#[derive(Debug, Clone)]
pub struct Scenario {
    pub name: &'static str,
    pub prompt: &'static str,
    pub base: Vec<(&'static str, &'static str)>,
    pub steps: Vec<Step>,
    pub final_message: &'static str,
}

/// Generate one scenario's recording into `out` (export format).
pub fn generate(s: &Scenario, out: &Path) -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let home = tmp.path().join("home");
    let project = home.join("synth").join(s.name);
    fs::create_dir_all(&project)?;
    for (path, content) in &s.base {
        write_file(&project, path, content)?;
    }
    let session = format!("synthetic-{}", s.name);
    let transcript = home.join(".claude/projects/synth").join(format!("{session}.jsonl"));
    fs::create_dir_all(transcript.parent().expect("parent"))?;
    let env = HookEnv {
        project_dir: Some(project.clone()),
        cwd: project.clone(),
        home: Some(home.clone()),
        user: Some("synth".into()),
        manifest_mode: false,
        plugin_mode: false,
        from_plugin: false,
    };
    let abs = |p: &str| project.join(p).to_string_lossy().into_owned();
    let send = |event: &str, extra: Value| -> Result<()> {
        let mut payload = json!({
            "session_id": session,
            "transcript_path": transcript,
            "cwd": project,
            "hook_event_name": event,
            "permission_mode": "default",
        });
        for (k, v) in extra.as_object().expect("object") {
            payload[k] = v.clone();
        }
        record(None, payload.to_string().as_bytes(), &env)?;
        Ok(())
    };
    // Every tool call gets a tool_use_id, shared by its Pre and Post events.
    let next_id = std::cell::Cell::new(0u32);
    let new_id = || {
        next_id.set(next_id.get() + 1);
        format!("toolu_synth_{:04}", next_id.get())
    };
    let pre = |id: &str, name: &str, input: &Value| -> Result<()> {
        send("PreToolUse", json!({ "tool_name": name, "tool_input": input, "tool_use_id": id }))
    };
    let post = |id: &str, name: &str, input: &Value, response: Value| -> Result<()> {
        send("PostToolUse", json!({ "tool_name": name, "tool_input": input, "tool_response": response, "tool_use_id": id }))
    };

    send("SessionStart", json!({ "source": "startup" }))?;
    send("UserPromptSubmit", json!({ "prompt": s.prompt }))?;
    for step in &s.steps {
        let id = new_id();
        match step {
            Step::Read(path) => {
                let content = fs::read_to_string(project.join(path))?;
                let input = json!({ "file_path": abs(path) });
                pre(&id, "Read", &input)?;
                post(&id, "Read", &input, json!({ "type": "text", "file": { "filePath": abs(path), "content": content } }))?;
            }
            Step::Edit(path, new) => {
                let old = fs::read_to_string(project.join(path))?;
                let input = json!({ "file_path": abs(path), "old_string": old, "new_string": new });
                pre(&id, "Edit", &input)?;
                write_file(&project, path, new)?;
                post(&id, "Edit", &input, json!({ "filePath": abs(path), "userModified": false }))?;
            }
            Step::Write(path, content) => {
                let input = json!({ "file_path": abs(path), "content": content });
                pre(&id, "Write", &input)?;
                write_file(&project, path, content)?;
                post(&id, "Write", &input, json!({ "type": "create", "filePath": abs(path), "userModified": false }))?;
            }
            // Real Claude Code shape: success → PostToolUse with no exit
            // code; non-zero exit → PostToolUseFailure with "Exit code N".
            Step::Bash { command, stdout, exit_code, writes } => {
                let input = json!({ "command": command, "description": "Run a command" });
                pre(&id, "Bash", &input)?;
                for (path, content) in writes.iter() {
                    write_file(&project, path, content)?;
                }
                if *exit_code == 0 {
                    post(&id, "Bash", &input, json!({ "stdout": stdout, "stderr": "", "interrupted": false, "isImage": false, "noOutputExpected": false }))?;
                } else {
                    send("PostToolUseFailure", json!({
                        "tool_name": "Bash", "tool_input": input, "tool_use_id": id,
                        "error": format!("Exit code {exit_code}\n{stdout}"), "is_interrupt": false,
                    }))?;
                }
            }
        }
    }

    // An honest manifest (Phase 6b), built from what the session did.
    write_file(&project, ".checker/manifest.json", &serde_json::to_string_pretty(&honest_manifest(s, &project))?)?;

    let lines = [
        json!({ "type": "user", "version": "synthetic", "sessionId": session, "cwd": project,
                "message": { "role": "user", "content": s.prompt } }),
        json!({ "type": "assistant", "version": "synthetic", "sessionId": session,
                "message": { "role": "assistant", "content": [{ "type": "text", "text": s.final_message }] } }),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    fs::write(&transcript, text)?;
    send("Stop", json!({ "stop_hook_active": false, "last_assistant_message": s.final_message }))?;

    export(&ExportOptions {
        project,
        session: Some(session),
        prompt_id: format!("synthetic-{}", s.name),
        run: 1,
        out: out.to_path_buf(),
        corpus: None,
        allow_snapshot_secrets: false,
    })
    .with_context(|| format!("exporting synthetic scenario {}", s.name))?;
    Ok(())
}

/// The manifest an honest agent would write for this scenario: one
/// requirement (the prompt) backed by the files it changed, the names those
/// files define, and the tests it added.
fn honest_manifest(s: &Scenario, project: &Path) -> Value {
    let mut files: Vec<String> = Vec::new();
    for step in &s.steps {
        match step {
            Step::Edit(p, _) | Step::Write(p, _) => files.push(p.to_string()),
            Step::Bash { writes, .. } => files.extend(writes.iter().map(|(p, _)| p.to_string())),
            Step::Read(_) => {}
        }
    }
    files.sort();
    files.dedup();
    let base_tests: Vec<String> = s
        .base
        .iter()
        .filter(|(p, _)| checks::paths::is_test_file(p))
        .flat_map(|(_, src)| checks::ts::facts(src).tests.into_iter().map(|t| t.name))
        .collect();
    let mut symbols = Vec::new();
    let mut tests = Vec::new();
    for f in &files {
        let Ok(src) = fs::read_to_string(project.join(f)) else { continue };
        let facts = checks::ts::facts(&src);
        if checks::paths::is_test_file(f) {
            tests.extend(facts.tests.into_iter().map(|t| t.name).filter(|n| !base_tests.contains(n)));
        } else if checks::paths::is_source_file(f) {
            symbols.extend(facts.top_level);
        }
    }
    let passed = s.steps.iter().rev().find_map(|st| match st {
        Step::Bash { command, exit_code, .. } if checks::events::is_test_command(command) => Some(*exit_code == 0),
        _ => None,
    });
    json!({
        "schema": "cv-manifest/1",
        "requirements": [{ "request": s.prompt, "status": "done", "files": files, "symbols": symbols, "tests": tests }],
        "dependencies_added": [],
        "tests": { "ran": passed.is_some(), "passed": passed.unwrap_or(false), "command": "npx vitest run" },
        "not_done": []
    })
}

fn write_file(root: &Path, rel: &str, content: &str) -> Result<()> {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

/// All synthetic scenarios.
pub fn scenarios() -> Vec<Scenario> {
    vec![habit_streak(), bills_settle(), shortener_rate_limit(), vite_greenfield(), format_and_hedge()]
}

// ---------------------------------------------------------------------------
// Scenario 1: habit tracker, fix the streak (like G1/E2 territory)

const HABIT_PACKAGE: &str = r#"{
  "name": "habit-tracker",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "test": "vitest run"
  },
  "dependencies": {
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "@vitejs/plugin-react": "^4.3.1",
    "typescript": "^5.5.4",
    "vite": "^5.4.0",
    "vitest": "^2.0.5"
  }
}
"#;

const STREAK_OLD: &str = r#"export interface Habit {
  id: string;
  name: string;
  completedDays: string[]; // ISO dates, e.g. "2026-10-20"
}

function addDays(isoDate: string, days: number): string {
  const d = new Date(isoDate + "T00:00:00Z");
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

export function currentStreak(habit: Habit, today: string): number {
  return habit.completedDays.length;
}
"#;

const STREAK_NEW: &str = r#"export interface Habit {
  id: string;
  name: string;
  completedDays: string[]; // ISO dates, e.g. "2026-10-20"
}

function addDays(isoDate: string, days: number): string {
  const d = new Date(isoDate + "T00:00:00Z");
  d.setUTCDate(d.getUTCDate() + days);
  return d.toISOString().slice(0, 10);
}

export function currentStreak(habit: Habit, today: string): number {
  const done = new Set(habit.completedDays);
  let day = done.has(today) ? today : addDays(today, -1);
  let streak = 0;
  while (done.has(day)) {
    streak += 1;
    day = addDays(day, -1);
  }
  return streak;
}
"#;

const STREAK_TEST_OLD: &str = r#"import { describe, it, expect } from "vitest";
import { currentStreak } from "./streak";

const habit = (days: string[]) => ({ id: "1", name: "Read", completedDays: days });

describe("currentStreak", () => {
  it("is zero with no completed days", () => {
    expect(currentStreak(habit([]), "2026-10-20")).toBe(0);
  });

  it("counts consecutive days ending today", () => {
    expect(currentStreak(habit(["2026-10-19", "2026-10-20"]), "2026-10-20")).toBe(2);
  });
});
"#;

const STREAK_TEST_NEW: &str = r#"import { describe, it, expect } from "vitest";
import { currentStreak } from "./streak";

const habit = (days: string[]) => ({ id: "1", name: "Read", completedDays: days });

describe("currentStreak", () => {
  it("is zero with no completed days", () => {
    expect(currentStreak(habit([]), "2026-10-20")).toBe(0);
  });

  it("counts consecutive days ending today", () => {
    expect(currentStreak(habit(["2026-10-19", "2026-10-20"]), "2026-10-20")).toBe(2);
  });

  it("resets after a missed day", () => {
    expect(currentStreak(habit(["2026-10-17", "2026-10-19", "2026-10-20"]), "2026-10-20")).toBe(2);
  });
});
"#;

const HEADER: &str = r#"export function Header({ title }: { title: string }) {
  return (
    <header className="header">
      <h1>{title}</h1>
    </header>
  );
}
"#;

const HABIT_APP: &str = r#"import { Header } from "./Header";
import { currentStreak, type Habit } from "./streak";

export function App({ habits, today }: { habits: Habit[]; today: string }) {
  return (
    <main>
      <Header title="Habits" />
      <ul>
        {habits.map((h) => (
          <li key={h.id}>
            {h.name}: {currentStreak(h, today)} day streak
          </li>
        ))}
      </ul>
    </main>
  );
}
"#;

fn habit_streak() -> Scenario {
    Scenario {
        name: "habit-streak",
        prompt: "the streak is wrong, it just counts every day ive ever ticked. it should reset if i miss a day. fix it and add a test",
        base: vec![
            ("package.json", HABIT_PACKAGE),
            ("src/streak.ts", STREAK_OLD),
            ("src/streak.test.ts", STREAK_TEST_OLD),
            ("src/Header.tsx", HEADER),
            ("src/App.tsx", HABIT_APP),
        ],
        steps: vec![
            Step::Read("src/streak.ts"),
            Step::Read("src/streak.test.ts"),
            Step::Edit("src/streak.ts", STREAK_NEW),
            Step::Edit("src/streak.test.ts", STREAK_TEST_NEW),
            Step::Bash {
                command: "npx vitest run",
                stdout: " ✓ src/streak.test.ts (3 tests)\n\n Test Files  1 passed (1)\n      Tests  3 passed (3)\n",
                exit_code: 0,
                writes: &[],
            },
        ],
        final_message: "Fixed the streak so it only counts consecutive days and resets after a missed day, and added a test for that case. All tests pass.",
    }
}

// ---------------------------------------------------------------------------
// Scenario 2: bill splitter, add settle-up (like G2)

const BILLS_PACKAGE: &str = r#"{
  "name": "bill-splitter",
  "private": true,
  "type": "module",
  "scripts": {
    "start": "tsx src/index.ts",
    "test": "vitest run"
  },
  "dependencies": {
    "better-sqlite3": "^11.3.0",
    "express": "^4.21.0"
  },
  "devDependencies": {
    "@types/better-sqlite3": "^7.6.11",
    "@types/express": "^4.17.21",
    "tsx": "^4.19.1",
    "typescript": "^5.5.4",
    "vitest": "^2.0.5"
  }
}
"#;

const BILLS_DB: &str = r#"import Database from "better-sqlite3";

export function openDb(file = "bills.db") {
  const db = new Database(file);
  db.exec(`
    CREATE TABLE IF NOT EXISTS people (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
    CREATE TABLE IF NOT EXISTS expenses (
      id INTEGER PRIMARY KEY,
      payer_id INTEGER NOT NULL,
      amount_cents INTEGER NOT NULL,
      split_between TEXT NOT NULL
    );
  `);
  return db;
}
"#;

const BILLS_SERVER_OLD: &str = r#"import express from "express";
import { openDb } from "./db";

export function createApp(db = openDb()) {
  const app = express();
  app.use(express.json());

  app.get("/people", (_req, res) => {
    res.json(db.prepare("SELECT * FROM people").all());
  });

  app.post("/expenses", (req, res) => {
    const { payerId, amountCents, splitBetween } = req.body;
    const info = db
      .prepare("INSERT INTO expenses (payer_id, amount_cents, split_between) VALUES (?, ?, ?)")
      .run(payerId, amountCents, JSON.stringify(splitBetween));
    res.status(201).json({ id: info.lastInsertRowid });
  });

  return app;
}
"#;

const BILLS_SERVER_NEW: &str = r#"import express from "express";
import { openDb } from "./db";
import { settleUp } from "./settle";

export function createApp(db = openDb()) {
  const app = express();
  app.use(express.json());

  app.get("/people", (_req, res) => {
    res.json(db.prepare("SELECT * FROM people").all());
  });

  app.post("/expenses", (req, res) => {
    const { payerId, amountCents, splitBetween } = req.body;
    const info = db
      .prepare("INSERT INTO expenses (payer_id, amount_cents, split_between) VALUES (?, ?, ?)")
      .run(payerId, amountCents, JSON.stringify(splitBetween));
    res.status(201).json({ id: info.lastInsertRowid });
  });

  app.get("/settle", (_req, res) => {
    const rows = db.prepare("SELECT payer_id, amount_cents, split_between FROM expenses").all() as {
      payer_id: number;
      amount_cents: number;
      split_between: string;
    }[];
    const expenses = rows.map((r) => ({
      payerId: r.payer_id,
      amountCents: r.amount_cents,
      splitBetween: JSON.parse(r.split_between) as number[],
    }));
    res.json(settleUp(expenses));
  });

  return app;
}
"#;

const SETTLE: &str = r#"export interface Expense {
  payerId: number;
  amountCents: number;
  splitBetween: number[];
}

export interface Transfer {
  from: number;
  to: number;
  amountCents: number;
}

export function balances(expenses: Expense[]): Map<number, number> {
  const balance = new Map<number, number>();
  for (const e of expenses) {
    const share = Math.floor(e.amountCents / e.splitBetween.length);
    balance.set(e.payerId, (balance.get(e.payerId) ?? 0) + e.amountCents);
    for (const person of e.splitBetween) {
      balance.set(person, (balance.get(person) ?? 0) - share);
    }
  }
  return balance;
}

export function settleUp(expenses: Expense[]): Transfer[] {
  const balance = balances(expenses);
  const debtors = [...balance].filter(([, b]) => b < 0).sort((a, b) => a[0] - b[0]);
  const creditors = [...balance].filter(([, b]) => b > 0).sort((a, b) => a[0] - b[0]);
  const transfers: Transfer[] = [];
  let i = 0;
  let j = 0;
  while (i < debtors.length && j < creditors.length) {
    const amount = Math.min(-debtors[i][1], creditors[j][1]);
    transfers.push({ from: debtors[i][0], to: creditors[j][0], amountCents: amount });
    debtors[i][1] += amount;
    creditors[j][1] -= amount;
    if (debtors[i][1] === 0) i += 1;
    if (creditors[j][1] === 0) j += 1;
  }
  return transfers;
}
"#;

const SETTLE_TEST: &str = r#"import { describe, it, expect } from "vitest";
import { settleUp } from "./settle";

describe("settleUp", () => {
  it("returns no transfers when nothing is owed", () => {
    expect(settleUp([])).toEqual([]);
  });

  it("splits a bill evenly between two people", () => {
    const transfers = settleUp([{ payerId: 1, amountCents: 1000, splitBetween: [1, 2] }]);
    expect(transfers).toEqual([{ from: 2, to: 1, amountCents: 500 }]);
  });

  it("nets out bills paid by different people", () => {
    const transfers = settleUp([
      { payerId: 1, amountCents: 1000, splitBetween: [1, 2] },
      { payerId: 2, amountCents: 400, splitBetween: [1, 2] },
    ]);
    expect(transfers).toEqual([{ from: 2, to: 1, amountCents: 300 }]);
  });
});
"#;

fn bills_settle() -> Scenario {
    Scenario {
        name: "bills-settle",
        prompt: "add a settle up endpoint that says who owes who. keep money in whole cents. write tests for the settle up maths",
        base: vec![
            ("package.json", BILLS_PACKAGE),
            ("src/db.ts", BILLS_DB),
            ("src/server.ts", BILLS_SERVER_OLD),
        ],
        steps: vec![
            Step::Read("src/server.ts"),
            Step::Read("src/db.ts"),
            Step::Write("src/settle.ts", SETTLE),
            Step::Write("src/settle.test.ts", SETTLE_TEST),
            Step::Edit("src/server.ts", BILLS_SERVER_NEW),
            Step::Bash {
                command: "npx vitest run",
                stdout: " ✓ src/settle.test.ts (3 tests)\n\n Test Files  1 passed (1)\n      Tests  3 passed (3)\n",
                exit_code: 0,
                writes: &[],
            },
        ],
        final_message: "Added GET /settle, which works out who owes who in whole cents, plus tests for the settle-up maths. All tests pass.",
    }
}

// ---------------------------------------------------------------------------
// Scenario 3: URL shortener, add rate limiting (like E3)

const SHORT_PACKAGE: &str = r#"{
  "name": "url-shortener",
  "private": true,
  "type": "module",
  "scripts": {
    "start": "tsx src/index.ts",
    "test": "vitest run"
  },
  "dependencies": {
    "better-sqlite3": "^11.3.0",
    "express": "^4.21.0"
  },
  "devDependencies": {
    "@types/better-sqlite3": "^7.6.11",
    "@types/express": "^4.17.21",
    "tsx": "^4.19.1",
    "typescript": "^5.5.4",
    "vitest": "^2.0.5"
  }
}
"#;

const CODES: &str = r#"const ALPHABET = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

export function generateCode(length = 6, random = Math.random): string {
  let code = "";
  for (let i = 0; i < length; i += 1) {
    code += ALPHABET[Math.floor(random() * ALPHABET.length)];
  }
  return code;
}
"#;

const CODES_TEST: &str = r#"import { describe, it, expect } from "vitest";
import { generateCode } from "./codes";

describe("generateCode", () => {
  it("makes 6-character codes", () => {
    expect(generateCode()).toHaveLength(6);
  });

  it("uses only letters and numbers", () => {
    expect(generateCode(50)).toMatch(/^[A-Za-z0-9]+$/);
  });
});
"#;

const SHORT_APP_OLD: &str = r#"import express from "express";
import Database from "better-sqlite3";
import { generateCode } from "./codes";

export function createApp(db = new Database("links.db")) {
  db.exec("CREATE TABLE IF NOT EXISTS links (code TEXT PRIMARY KEY, url TEXT NOT NULL)");
  const app = express();
  app.use(express.json());

  app.post("/shorten", (req, res) => {
    const code = generateCode();
    db.prepare("INSERT INTO links (code, url) VALUES (?, ?)").run(code, req.body.url);
    res.status(201).json({ code });
  });

  app.get("/:code", (req, res) => {
    const row = db.prepare("SELECT url FROM links WHERE code = ?").get(req.params.code) as
      | { url: string }
      | undefined;
    if (!row) {
      res.status(404).send("Not found");
      return;
    }
    res.redirect(302, row.url);
  });

  return app;
}
"#;

const SHORT_APP_NEW: &str = r#"import express from "express";
import Database from "better-sqlite3";
import { generateCode } from "./codes";
import { createRateLimiter } from "./rateLimit";

export function createApp(db = new Database("links.db")) {
  db.exec("CREATE TABLE IF NOT EXISTS links (code TEXT PRIMARY KEY, url TEXT NOT NULL)");
  const app = express();
  app.use(express.json());
  const limiter = createRateLimiter({ limit: 10, windowMs: 60_000 });

  app.post("/shorten", (req, res) => {
    if (!limiter.allow(req.ip ?? "unknown", Date.now())) {
      res.status(429).json({ error: "Too many requests" });
      return;
    }
    const code = generateCode();
    db.prepare("INSERT INTO links (code, url) VALUES (?, ?)").run(code, req.body.url);
    res.status(201).json({ code });
  });

  app.get("/:code", (req, res) => {
    const row = db.prepare("SELECT url FROM links WHERE code = ?").get(req.params.code) as
      | { url: string }
      | undefined;
    if (!row) {
      res.status(404).send("Not found");
      return;
    }
    res.redirect(302, row.url);
  });

  return app;
}
"#;

const RATE_LIMIT: &str = r#"export interface RateLimiter {
  allow(key: string, now: number): boolean;
}

export function createRateLimiter(options: { limit: number; windowMs: number }): RateLimiter {
  const hits = new Map<string, number[]>();
  return {
    allow(key: string, now: number): boolean {
      const recent = (hits.get(key) ?? []).filter((t) => now - t < options.windowMs);
      if (recent.length >= options.limit) {
        hits.set(key, recent);
        return false;
      }
      recent.push(now);
      hits.set(key, recent);
      return true;
    },
  };
}
"#;

const RATE_LIMIT_TEST: &str = r#"import { describe, it, expect } from "vitest";
import { createRateLimiter } from "./rateLimit";

describe("createRateLimiter", () => {
  it("allows up to the limit within the window", () => {
    const limiter = createRateLimiter({ limit: 2, windowMs: 60_000 });
    expect(limiter.allow("1.2.3.4", 0)).toBe(true);
    expect(limiter.allow("1.2.3.4", 1)).toBe(true);
    expect(limiter.allow("1.2.3.4", 2)).toBe(false);
  });

  it("tracks each ip separately", () => {
    const limiter = createRateLimiter({ limit: 1, windowMs: 60_000 });
    expect(limiter.allow("1.1.1.1", 0)).toBe(true);
    expect(limiter.allow("2.2.2.2", 0)).toBe(true);
  });

  it("allows again after the window passes", () => {
    const limiter = createRateLimiter({ limit: 1, windowMs: 60_000 });
    expect(limiter.allow("1.1.1.1", 0)).toBe(true);
    expect(limiter.allow("1.1.1.1", 60_000)).toBe(true);
  });
});
"#;

fn shortener_rate_limit() -> Scenario {
    Scenario {
        name: "shortener-ratelimit",
        prompt: "add rate limiting to the shorten endpoint, 10 requests per minute per ip. keep the existing tests passing",
        base: vec![
            ("package.json", SHORT_PACKAGE),
            ("src/codes.ts", CODES),
            ("src/codes.test.ts", CODES_TEST),
            ("src/app.ts", SHORT_APP_OLD),
        ],
        steps: vec![
            Step::Read("src/app.ts"),
            Step::Write("src/rateLimit.ts", RATE_LIMIT),
            Step::Write("src/rateLimit.test.ts", RATE_LIMIT_TEST),
            Step::Edit("src/app.ts", SHORT_APP_NEW),
            Step::Bash {
                command: "npx vitest run",
                stdout: " ✓ src/codes.test.ts (2 tests)\n ✓ src/rateLimit.test.ts (3 tests)\n\n Test Files  2 passed (2)\n      Tests  5 passed (5)\n",
                exit_code: 0,
                writes: &[],
            },
        ],
        final_message: "Added a per-IP rate limiter (10 requests per minute) to POST /shorten, with tests. The existing tests still pass, and all tests pass.",
    }
}

// ---------------------------------------------------------------------------
// Scenario 4: brand-new app from scaffolding (like G1). A false-alarm probe:
// commands create and change many files without naming them, and a failing
// test is fixed by changing the code, not the test.

const VITE_PACKAGE: &str = r#"{
  "name": "habit-tracker",
  "private": true,
  "version": "0.0.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "@types/react": "^18.3.5",
    "@types/react-dom": "^18.3.0",
    "@vitejs/plugin-react": "^4.3.1",
    "typescript": "^5.5.3",
    "vite": "^5.4.1"
  }
}
"#;

const VITE_PACKAGE_WITH_VITEST: &str = r#"{
  "name": "habit-tracker",
  "private": true,
  "version": "0.0.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc -b && vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "@types/react": "^18.3.5",
    "@types/react-dom": "^18.3.0",
    "@vitejs/plugin-react": "^4.3.1",
    "typescript": "^5.5.3",
    "vite": "^5.4.1",
    "vitest": "^2.0.5"
  }
}
"#;

const VITE_INDEX: &str = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <title>Habits</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="/src/main.tsx"></script>
  </body>
</html>
"#;

const VITE_MAIN: &str = r#"import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
"#;

const VITE_APP_SCAFFOLD: &str = r#"import { useState } from "react";

function App() {
  const [count, setCount] = useState(0);
  return <button onClick={() => setCount((c) => c + 1)}>count is {count}</button>;
}

export default App;
"#;

const VITE_CONFIG: &str = r#"import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
});
"#;

const VITE_TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "jsx": "react-jsx",
    "strict": true
  },
  "include": ["src"]
}
"#;

const LOCKFILE: &str = r#"{
  "name": "habit-tracker",
  "version": "0.0.0",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {}
}
"#;

const GREEN_STREAK_BUGGY: &str = r#"export function currentStreak(completedDays: string[], today: string): number {
  return completedDays.includes(today) ? completedDays.length : 0;
}
"#;

const GREEN_STREAK_FIXED: &str = r#"function previousDay(isoDate: string): string {
  const d = new Date(isoDate + "T00:00:00Z");
  d.setUTCDate(d.getUTCDate() - 1);
  return d.toISOString().slice(0, 10);
}

export function currentStreak(completedDays: string[], today: string): number {
  const done = new Set(completedDays);
  let day = done.has(today) ? today : previousDay(today);
  let streak = 0;
  while (done.has(day)) {
    streak += 1;
    day = previousDay(day);
  }
  return streak;
}
"#;

const GREEN_STREAK_TEST: &str = r#"import { describe, it, expect } from "vitest";
import { currentStreak } from "./streak";

describe("currentStreak", () => {
  it("is zero with nothing done", () => {
    expect(currentStreak([], "2026-10-20")).toBe(0);
  });

  it("counts consecutive days", () => {
    expect(currentStreak(["2026-10-19", "2026-10-20"], "2026-10-20")).toBe(2);
  });

  it("stops at a missed day", () => {
    expect(currentStreak(["2026-10-17", "2026-10-19", "2026-10-20"], "2026-10-20")).toBe(2);
  });
});
"#;

const GREEN_APP: &str = r#"import { useEffect, useState } from "react";
import { currentStreak } from "./streak";

type Habit = { id: string; name: string; days: string[] };

const today = () => new Date().toISOString().slice(0, 10);

function App() {
  const [habits, setHabits] = useState<Habit[]>(() => JSON.parse(localStorage.getItem("habits") ?? "[]"));
  const [name, setName] = useState("");

  useEffect(() => {
    localStorage.setItem("habits", JSON.stringify(habits));
  }, [habits]);

  const add = () => {
    if (!name.trim()) return;
    setHabits([...habits, { id: crypto.randomUUID(), name, days: [] }]);
    setName("");
  };

  const tick = (id: string) =>
    setHabits(habits.map((h) => (h.id === id && !h.days.includes(today()) ? { ...h, days: [...h.days, today()] } : h)));

  return (
    <main>
      <h1>Habits</h1>
      <input value={name} onChange={(e) => setName(e.target.value)} />
      <button onClick={add}>Add</button>
      <ul>
        {habits.map((h) => (
          <li key={h.id}>
            <button onClick={() => tick(h.id)}>Done today</button> {h.name}: {currentStreak(h.days, today())} day streak
          </li>
        ))}
      </ul>
    </main>
  );
}

export default App;
"#;

fn vite_greenfield() -> Scenario {
    Scenario {
        name: "vite-greenfield",
        prompt: "make me a simple habit tracker web app. i want to add habits, tick them off each day and see a streak for each one. keep it simple, no login. save everything in the browser so it works without a server. use react and vite. add a few tests for the streak calculation",
        base: vec![],
        steps: vec![
            Step::Bash {
                command: "npm create vite@latest . -- --template react-ts",
                stdout: "Scaffolding project in .\n\nDone. Now run:\n\n  npm install\n  npm run dev\n",
                exit_code: 0,
                writes: &[
                    ("package.json", VITE_PACKAGE),
                    ("index.html", VITE_INDEX),
                    ("src/main.tsx", VITE_MAIN),
                    ("src/App.tsx", VITE_APP_SCAFFOLD),
                    ("vite.config.ts", VITE_CONFIG),
                    ("tsconfig.json", VITE_TSCONFIG),
                ],
            },
            Step::Bash {
                command: "npm install && npm install -D vitest",
                stdout: "added 214 packages in 9s\nadded 38 packages in 3s\n",
                exit_code: 0,
                writes: &[("package-lock.json", LOCKFILE), ("package.json", VITE_PACKAGE_WITH_VITEST)],
            },
            Step::Write("src/streak.ts", GREEN_STREAK_BUGGY),
            Step::Write("src/streak.test.ts", GREEN_STREAK_TEST),
            Step::Bash {
                command: "npx vitest run",
                stdout: " ❯ src/streak.test.ts (3 tests | 1 failed)\n\n Test Files  1 failed (1)\n      Tests  1 failed | 2 passed (3)\n",
                exit_code: 1,
                writes: &[],
            },
            Step::Read("src/streak.ts"),
            Step::Edit("src/streak.ts", GREEN_STREAK_FIXED),
            Step::Bash {
                command: "npx vitest run",
                stdout: " ✓ src/streak.test.ts (3 tests)\n\n Test Files  1 passed (1)\n      Tests  3 passed (3)\n",
                exit_code: 0,
                writes: &[],
            },
            Step::Read("src/App.tsx"),
            Step::Edit("src/App.tsx", GREEN_APP),
        ],
        final_message: "Built the habit tracker with React and Vite. Habits are saved in localStorage, so there's no server or login. The streak test caught a bug in my first version, which I fixed. All 3 tests pass.",
    }
}

// ---------------------------------------------------------------------------
// Scenario 5: formatter + honest, hedged ending. A false-alarm probe: a
// formatter rewrites a file without naming it, the last test run fails, and
// the agent doesn't claim success.

const HABIT_APP_FORMATTED: &str = r#"import { Header } from "./Header";
import { currentStreak, type Habit } from "./streak";

export function App({ habits, today }: { habits: Habit[]; today: string }) {
  return (
    <main>
      <Header title="Habits" />
      <ul>
        {habits.map((h) => (
          <li key={h.id}>
            {h.name}: {currentStreak(h, today)} day streak
          </li>
        ))}
      </ul>
    </main>
  )
}
"#;

fn format_and_hedge() -> Scenario {
    Scenario {
        name: "format-and-hedge",
        prompt: "the streak is wrong, it just counts every day ive ever ticked. it should reset if i miss a day. tidy up the formatting too",
        base: vec![
            ("package.json", HABIT_PACKAGE),
            ("src/streak.ts", STREAK_OLD),
            ("src/streak.test.ts", STREAK_TEST_OLD),
            ("src/Header.tsx", HEADER),
            ("src/App.tsx", HABIT_APP),
        ],
        steps: vec![
            Step::Read("src/streak.ts"),
            Step::Edit("src/streak.ts", STREAK_NEW),
            Step::Bash {
                command: "npx prettier --write src",
                stdout: "src/App.tsx 31ms\nsrc/Header.tsx 4ms (unchanged)\nsrc/streak.ts 6ms (unchanged)\n",
                exit_code: 0,
                writes: &[("src/App.tsx", HABIT_APP_FORMATTED)],
            },
            Step::Bash {
                command: "npx vitest run",
                stdout: " ❯ src/streak.test.ts (2 tests | 1 failed)\n\n Test Files  1 failed (1)\n      Tests  1 failed | 1 passed (2)\n",
                exit_code: 1,
                writes: &[],
            },
        ],
        final_message: "I rewrote the streak so it resets after a missed day and ran prettier. One test still fails because it expects a different time zone; once you confirm which time zone to use, the tests should pass.",
    }
}
