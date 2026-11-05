import express from "express";
import { generateCode, isValidCode } from "./code.js";
import { UrlStore, type Db } from "./db.js";

const MAX_ATTEMPTS = 10;
const MAX_URL_LENGTH = 2048;

export interface AppOptions {
  /** Base used to build the returned short link, e.g. "https://sho.rt". Defaults to the request's host. */
  baseUrl?: string;
  /** Override for tests. */
  generateCode?: () => string;
}

function isHttpUrl(value: unknown): value is string {
  if (typeof value !== "string" || value.length >= MAX_URL_LENGTH) return false;
  try {
    const { protocol } = new URL(value);
    return protocol === "http:" || protocol === "https:";
  } catch {
    return false;
  }
}

export function createApp(db: Db, options: AppOptions = {}) {
  const store = new UrlStore(db);
  const makeCode = options.generateCode ?? generateCode;
  const app = express();

  app.use(express.json({ limit: "10kb" }));

  app.post("/api/shorten", (req, res) => {
    const url = req.body?.url;
    if (!isHttpUrl(url)) {
      res.status(400).json({ error: "Body must include a valid http(s) 'url'" });
      return;
    }

    for (let attempt = 0; attempt < MAX_ATTEMPTS; attempt++) {
      const code = makeCode();
      if (store.insert(code, url)) {
        const base = options.baseUrl ?? `${req.protocol}://${req.get("host")}`;
        res.status(201).json({ code, shortUrl: `${base}/${code}`, url });
        return;
      }
    }

    res.status(503).json({ error: "Could not allocate a short code, try again" });
  });

  app.get("/:code", (req, res) => {
    const { code } = req.params;
    const url = isValidCode(code) ? store.find(code) : undefined;
    if (!url) {
      res.status(404).json({ error: "Short link not found" });
      return;
    }
    res.redirect(302, url);
  });

  // Malformed JSON bodies and other errors
  app.use(
    (err: Error & { status?: number }, _req: express.Request, res: express.Response, _next: express.NextFunction) => {
      const status = err.status && err.status < 500 ? err.status : 500;
      res.status(status).json({ error: status === 500 ? "Internal server error" : err.message });
    },
  );

  return app;
}
