# Notes API

Build a small notes service: a REST API over HTTP with JSON in and JSON out. Use Node.js 18 (plain JS or TypeScript, your choice). `npm start` must start the server; you may use any npm packages you like, but the built-in `http` module is enough.

## Runtime
- `npm start` starts the server on the port in the `PORT` environment variable.
- Everything that must survive a restart is stored under the directory in the `DATA_DIR` environment variable (it exists and starts empty). Write nothing outside it.
- `GET /health` returns `200` with `{"ok": true}` once the server is ready.
- Every response body is JSON. Errors are `{"error": "<message>"}` with a 4xx status: `400` for invalid input (including a request body that isn't valid JSON), `404` for an unknown note or an unknown route, `409` where stated below.

## Notes
A note is returned as:
```json
{ "id": "…", "title": "…", "body": "…", "tags": ["…"], "createdAt": "<ISO 8601>", "updatedAt": "<ISO 8601>", "deletedAt": null }
```
- `id` is a string, unique for the lifetime of the data (never reused, not even after a delete or a restart).
- `deletedAt` is `null` for a live note and an ISO 8601 timestamp for a soft-deleted one.

### Create: `POST /notes`
Body: `{ "title": string, "body"?: string, "tags"?: string[] }`. Responds `201` with the note.
- `title` is required, must be a string, is trimmed, and must not be empty after trimming or longer than 200 characters.
- `body` defaults to `""`; if given it must be a string of at most 10000 characters.
- `tags` defaults to `[]`; if given it must be an array of strings. Each tag is trimmed and lower-cased; empty tags (after trimming) are invalid; a tag may be at most 30 characters. Duplicates are removed, keeping the first occurrence and the original order. After removing duplicates there may be at most 10 tags.
- Any rule broken gives `400`.

### Read: `GET /notes/:id`
`200` with the note, or `404` if it doesn't exist or is soft-deleted.

### Update: `PATCH /notes/:id`
Body: any of `title`, `body`, `tags` (same rules as create). Only the given fields change. An empty object `{}` is allowed and changes nothing. `updatedAt` is set to the current time whenever the request succeeds. `200` with the updated note; `404` if the note doesn't exist or is soft-deleted; `400` for invalid input.

### Delete: `DELETE /notes/:id`
Soft delete: sets `deletedAt`. Responds `204` with no body. `404` if the note doesn't exist or is already soft-deleted.

### Restore: `POST /notes/:id/restore`
Clears `deletedAt` and responds `200` with the note. `404` if the note doesn't exist, `409` if it isn't soft-deleted.

### List and search: `GET /notes`
Responds `200` with `{ "items": [note, …], "total": <number>, "limit": <number>, "offset": <number> }`.
- Newest first: the most recently created note comes first (order of creation, not `updatedAt`).
- Query parameters (all optional):
  - `q`: keep notes whose title or body contains `q`, case-insensitively. An empty `q` matches everything.
  - `tag`: keep notes carrying that tag (compare after trim + lower-casing). The parameter may be repeated (`?tag=a&tag=b`); a note must carry all of the given tags.
  - `deleted`: `exclude` (default) lists live notes only, `only` lists only soft-deleted notes, `include` lists both. Any other value is a `400`.
  - `limit`: an integer from 1 to 100, default 20. `offset`: an integer of 0 or more, default 0. Anything else is a `400`.
- `total` is the number of notes matching the filters, before `limit` and `offset` are applied. `limit` and `offset` in the response echo the values used.

### Tags: `GET /tags`
Responds `200` with `{ "items": [{ "tag": "…", "count": <number> }, …] }`, counting live (not soft-deleted) notes only, sorted by `count` descending and then by `tag` ascending. Tags with a count of 0 are left out.

## Persistence
Notes (including soft-deleted ones and their ids) must still be there after the server is stopped and started again with the same `DATA_DIR`. Ids created after a restart must not clash with earlier ones.
