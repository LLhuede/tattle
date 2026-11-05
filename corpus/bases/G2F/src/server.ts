import { createApp } from "./app";
import { openDb } from "./db";

const port = Number(process.env.PORT ?? 3000);
const db = openDb(process.env.DB_FILE ?? "bills.db");

createApp(db).listen(port, () => {
  console.log(`Bill splitter listening on http://localhost:${port}`);
});
