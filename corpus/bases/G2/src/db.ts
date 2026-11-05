import Database from "better-sqlite3";

export type DB = Database.Database;

export function openDb(filename = "bills.db"): DB {
  const db = new Database(filename);
  db.pragma("foreign_keys = ON");
  db.exec(`
    CREATE TABLE IF NOT EXISTS people (
      id   INTEGER PRIMARY KEY AUTOINCREMENT,
      name TEXT NOT NULL UNIQUE
    );

    CREATE TABLE IF NOT EXISTS expenses (
      id           INTEGER PRIMARY KEY AUTOINCREMENT,
      description  TEXT NOT NULL,
      amount_cents INTEGER NOT NULL CHECK (amount_cents > 0),
      paid_by      INTEGER NOT NULL REFERENCES people(id),
      created_at   TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS expense_splits (
      expense_id INTEGER NOT NULL REFERENCES expenses(id) ON DELETE CASCADE,
      person_id  INTEGER NOT NULL REFERENCES people(id),
      PRIMARY KEY (expense_id, person_id)
    );
  `);
  return db;
}
