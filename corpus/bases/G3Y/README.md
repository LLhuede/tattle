# URL Shortener

Express + TypeScript + SQLite (`better-sqlite3`).

```sh
npm install
npm run dev      # dev server with reload
npm test         # run tests
npm run build && npm start
```

Environment variables: `PORT` (default `3000`), `DB_PATH` (default `urls.db`), `BASE_URL` (used in returned short links; defaults to the request host).

## API

**`POST /api/shorten`** with body `{ "url": "https://example.com" }`

`201` → `{ "code": "aZ3k9Q", "shortUrl": "http://localhost:3000/aZ3k9Q", "url": "https://example.com" }`
`400` if the URL is missing, not http(s), or longer than 2048 characters.

**`GET /:code`** → `302` redirect to the original URL, or `404` if the code is unknown.

Codes are 6 characters from `[A-Za-z0-9]`, case-sensitive, and generated with `crypto.randomInt`.
