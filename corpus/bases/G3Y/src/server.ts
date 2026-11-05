import { createApp } from "./app.js";
import { openDb } from "./db.js";

const port = Number(process.env.PORT ?? 3000);
const db = openDb(process.env.DB_PATH ?? "urls.db");
const app = createApp(db, { baseUrl: process.env.BASE_URL });

app.listen(port, () => {
  console.log(`URL shortener listening on http://localhost:${port}`);
});
