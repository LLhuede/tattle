import express, { NextFunction, Request, Response } from "express";
import type { DB } from "./db";
import { computeBalances, settleUp } from "./settle";

class HttpError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

interface PersonRow {
  id: number;
  name: string;
}

interface ExpenseRow {
  id: number;
  description: string;
  amount_cents: number;
  paid_by: number;
  created_at: string;
}

const toDollars = (cents: number) => cents / 100;

/** Convert a dollar amount like 12.5 to cents, rejecting more than 2 decimal places. */
function toCents(amount: unknown): number {
  if (typeof amount !== "number" || !Number.isFinite(amount) || amount < 0) {
    throw new HttpError(400, "amount must be a positive number");
  }
  const cents = Math.round(amount * 100);
  if (Math.abs(cents - amount * 100) > 1e-6) {
    throw new HttpError(400, "amount can have at most 2 decimal places");
  }
  return cents;
}

export function createApp(db: DB) {
  const app = express();
  app.use(express.json());

  const getPerson = db.prepare<[number], PersonRow>("SELECT id, name FROM people WHERE id = ?");
  const getSplits = db.prepare<[number], { person_id: number }>(
    "SELECT person_id FROM expense_splits WHERE expense_id = ? ORDER BY person_id",
  );

  const expenseJson = (row: ExpenseRow) => ({
    id: row.id,
    description: row.description,
    amount: toDollars(row.amount_cents),
    paidBy: row.paid_by,
    splitBetween: getSplits.all(row.id).map((s) => s.person_id),
    createdAt: row.created_at,
  });

  const requirePeople = (ids: number[]) => {
    for (const id of ids) {
      if (!getPerson.get(id)) throw new HttpError(400, `person ${id} does not exist`);
    }
  };

  // ---- People ----

  app.get("/people", (_req, res) => {
    res.json(db.prepare("SELECT id, name FROM people ORDER BY id").all());
  });

  app.post("/people", (req, res) => {
    const name = typeof req.body?.name === "string" ? req.body.name.trim() : "";
    if (!name) throw new HttpError(400, "name is required");
    try {
      const { lastInsertRowid } = db.prepare("INSERT INTO people (name) VALUES (?)").run(name);
      res.status(201).json(getPerson.get(Number(lastInsertRowid)));
    } catch (err: any) {
      if (err.code === "SQLITE_CONSTRAINT_UNIQUE") {
        throw new HttpError(409, `a person named "${name}" already exists`);
      }
      throw err;
    }
  });

  // ---- Expenses ----

  app.get("/expenses", (_req, res) => {
    const rows = db.prepare<[], ExpenseRow>("SELECT * FROM expenses ORDER BY id").all();
    res.json(rows.map(expenseJson));
  });

  app.get("/expenses/:id", (req, res) => {
    const row = db
      .prepare<[number], ExpenseRow>("SELECT * FROM expenses WHERE id = ?")
      .get(Number(req.params.id));
    if (!row) throw new HttpError(404, "expense not found");
    res.json(expenseJson(row));
  });

  app.post("/expenses", (req, res) => {
    const { description, amount, paidBy, splitBetween } = req.body ?? {};

    if (typeof description !== "string" || !description.trim()) {
      throw new HttpError(400, "description is required");
    }
    const amountCents = toCents(amount);
    if (!Number.isInteger(paidBy)) throw new HttpError(400, "paidBy must be a person id");
    if (
      !Array.isArray(splitBetween) ||
      splitBetween.length === 0 ||
      !splitBetween.every(Number.isInteger)
    ) {
      throw new HttpError(400, "splitBetween must be a non-empty array of person ids");
    }
    const participants = [...new Set<number>(splitBetween)];
    requirePeople([paidBy, ...participants]);

    const insert = db.transaction(() => {
      const { lastInsertRowid } = db
        .prepare("INSERT INTO expenses (description, amount_cents, paid_by) VALUES (?, ?, ?)")
        .run(description.trim(), amountCents, paidBy);
      const id = Number(lastInsertRowid);
      const addSplit = db.prepare("INSERT INTO expense_splits (expense_id, person_id) VALUES (?, ?)");
      for (const personId of participants) addSplit.run(id, personId);
      return id;
    });
    const id = insert();

    const row = db.prepare<[number], ExpenseRow>("SELECT * FROM expenses WHERE id = ?").get(id)!;
    res.status(201).json(expenseJson(row));
  });

  app.delete("/expenses/:id", (req, res) => {
    const { changes } = db.prepare("DELETE FROM expenses WHERE id = ?").run(Number(req.params.id));
    if (changes === 0) throw new HttpError(404, "expense not found");
    res.status(204).end();
  });

  // ---- Settle up ----

  app.get("/settle-up", (_req, res) => {
    const people = db.prepare<[], PersonRow>("SELECT id, name FROM people ORDER BY id").all();
    const expenses = db
      .prepare<[], ExpenseRow>("SELECT * FROM expenses ORDER BY id")
      .all()
      .map((row) => ({
        amountCents: row.amount_cents,
        paidBy: row.paid_by,
        splitBetween: getSplits.all(row.id).map((s) => s.person_id),
      }));

    const names = new Map(people.map((p) => [p.id, p.name]));
    const balances = computeBalances(
      people.map((p) => p.id),
      expenses,
    );
    const transfers = settleUp(balances);

    res.json({
      balances: people.map((p) => ({
        personId: p.id,
        name: p.name,
        balance: toDollars(balances.get(p.id) ?? 0),
      })),
      transfers: transfers.map((t) => ({
        from: { id: t.from, name: names.get(t.from) },
        to: { id: t.to, name: names.get(t.to) },
        amount: toDollars(t.amountCents),
      })),
    });
  });

  // ---- Errors ----

  app.use((err: any, _req: Request, res: Response, _next: NextFunction) => {
    if (err instanceof HttpError) {
      res.status(err.status).json({ error: err.message });
    } else if (err?.type === "entity.parse.failed") {
      res.status(400).json({ error: "invalid JSON body" });
    } else {
      console.error(err);
      res.status(500).json({ error: "internal server error" });
    }
  });

  return app;
}
