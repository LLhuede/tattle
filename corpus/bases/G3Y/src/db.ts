import Database from "better-sqlite3";

export type Db = Database.Database;

export function openDb(filename: string): Db {
  const db = new Database(filename);
  db.pragma("journal_mode = WAL");
  db.exec(`
    CREATE TABLE IF NOT EXISTS urls (
      code       TEXT PRIMARY KEY,
      url        TEXT NOT NULL,
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    )
  `);
  return db;
}

export class UrlStore {
  private insertStmt;
  private findStmt;

  constructor(db: Db) {
    this.insertStmt = db.prepare("INSERT INTO urls (code, url) VALUES (?, ?)");
    this.findStmt = db.prepare("SELECT url FROM urls WHERE code = ?");
  }

  /** Returns false if the code is already taken. */
  insert(code: string, url: string): boolean {
    try {
      this.insertStmt.run(code, url);
      return true;
    } catch (err) {
      if ((err as { code?: string }).code === "SQLITE_CONSTRAINT_PRIMARYKEY") {
        return false;
      }
      throw err;
    }
  }

  find(code: string): string | undefined {
    const row = this.findStmt.get(code) as { url: string } | undefined;
    return row?.url;
  }
}
