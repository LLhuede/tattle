# Shared Expenses ("Splitwise-lite")

A small multi-user app for splitting shared costs. People sign up, make groups,
add each other, record who paid for what and how it is split, and the app shows
who owes whom and suggests the payments that settle everything up. It has an
HTTP JSON API and a small browser UI.

## Platform and contract

- Node.js 18 (there is no built-in SQLite on 18). TypeScript or plain JavaScript, your choice. Keep the dependency list short.
- `package.json` at the project root with `"type": "module"`.
- The project is installed with `npm install`, built with `npm run build` and started with `npm start`. `npm run build` must produce everything `npm start` and the UI need (compile the server if you use TypeScript, and build the UI with **Vite** into `dist/`).
- `npm start` starts the server. It listens on `process.env.PORT` (default 3000).
- All persistent data goes under the directory in `process.env.DATA_DIR` (it exists and is empty on first start). Write nothing outside it. Every write the API has acknowledged must survive the process being killed and restarted on the same `DATA_DIR`.
- `GET /health` returns `200` with `{"status":"ok"}` once the server is ready.

## API conventions

- Requests with a body are JSON (`Content-Type: application/json`). Responses are JSON.
- Errors are always JSON `{"error": "<message>"}` with a status code from this list:
  - `400` invalid input (including a body that is not valid JSON or not a JSON object)
  - `401` missing, malformed or unknown token
  - `403` signed in, but not allowed (for example not a member of the group)
  - `404` unknown resource or unknown route (any unknown path gets a JSON `404`, signed in or not)
  - `409` conflict (duplicate)
- Checks run in this order: route exists, then `401` (when the route needs a token), then `404` for the group (or other resource in the path), then `403` membership, then `400` for the body, then other `404`/`409`.
- Authentication: `Authorization: Bearer <token>`. Every route except `/health`, `POST /api/signup` and `POST /api/login` needs it.
- Ids are opaque non-empty strings. `createdAt` is an ISO-8601 UTC string (for example `2026-01-31T12:00:00.000Z`).
- Money is always an integer number of cents (`amountCents`). Never floats.
- Lists are wrapped in an object (for example `{"groups": [...]}`) so fields can be added later.

## Users and auth

User object: `{ "id", "email", "name" }`. It never contains a password or a hash, in any response.

- `POST /api/signup` body `{ "email", "name", "password" }` -> `201 { "token", "user" }`.
  - `email`: a string like `a@b.co` (must match `^[^\s@]+@[^\s@]+\.[^\s@]+$`, at most 254 chars). It is trimmed and lower-cased before it is stored and returned.
  - `name`: a string, 1 to 50 characters after trimming (the trimmed value is stored).
  - `password`: a string of 8 to 128 characters.
  - Invalid -> `400`. An email that already exists (compared lower-cased) -> `409`.
- `POST /api/login` body `{ "email", "password" }` -> `200 { "token", "user" }`. The email is compared lower-cased. Missing or non-string fields -> `400`. Wrong password or unknown email -> `401`.
- `GET /api/me` -> `200 { "user" }`.
- Every signup and login issues a new token; tokens of the same user are independent and all keep working.
- Passwords are stored hashed with a salt (for example `crypto.scrypt`), never as plain text, and nothing under `DATA_DIR` may contain a user's plain password.

## Groups

Group object: `{ "id", "name", "createdBy", "createdAt", "members" }` where `createdBy` is a user id and `members` is an array of user objects in the order they joined (the creator first).

- `POST /api/groups` body `{ "name" }` -> `201 <group>`. `name` is a string of 1 to 60 characters after trimming (the trimmed value is stored). The caller becomes the first member.
- `GET /api/groups` -> `200 { "groups": [<group>...] }`: only groups the caller is a member of, in creation order.
- `GET /api/groups/:groupId` -> `200 <group>`. Unknown group -> `404`; a group the caller is not a member of -> `403`.
- `POST /api/groups/:groupId/members` body `{ "email" }` -> `201 <group>` (the updated group). Any member can add. The email is compared lower-cased. No such user -> `404`; already a member -> `409`; a missing or non-string email -> `400`.

Everything below `/api/groups/:groupId/...` requires the caller to be a member (otherwise `403`; unknown group `404`).

## Expenses

Expense object:
```
{ "id", "groupId", "description", "amountCents", "paidBy",
  "split", "shares", "createdBy", "createdAt" }
```
- `paidBy`: the user id who paid. `createdBy`: the user id who entered it (may differ).
- `split` echoes what the client sent (see below). `shares` is always present: an object `{ "<userId>": <cents owed> }` with exactly one entry per participant, summing to `amountCents`.

`POST /api/groups/:groupId/expenses` body:
```
{ "description": "Dinner", "amountCents": 1000, "paidBy": "<userId>",
  "split": { "type": "equal", "participants": ["<userId>", ...] } }
```
or
```
  "split": { "type": "exact", "shares": { "<userId>": 400, "<userId>": 600 } }
```
-> `201 <expense>`.

Rules (any violation is `400`):
- `description`: string, 1 to 100 characters after trimming (the trimmed value is stored).
- `amountCents`: a positive integer (not a float, not a numeric string).
- `paidBy`: a member of the group. The payer does not have to be one of the participants.
- `split.type` is `"equal"` or `"exact"`.
- equal: `participants` is a non-empty array of distinct member ids. Each participant owes `floor(amountCents / n)` cents; the remainder `amountCents mod n` is handed out one cent each to the first participants in the order given in `participants`. Example: 1000 split among `[a, b, c]` gives a 334, b 333, c 333.
- exact: `shares` is a non-empty object whose keys are member ids and whose values are positive integers that sum exactly to `amountCents`. The participants are the keys.

Other routes:
- `GET /api/groups/:groupId/expenses` -> `200 { "expenses": [...] }`, oldest first (creation order).
- `GET /api/groups/:groupId/expenses/:expenseId` -> `200 <expense>` or `404`.
- `DELETE /api/groups/:groupId/expenses/:expenseId` -> `204` with no body. Only the member who created the expense may delete it (`403` for anyone else); unknown expense -> `404`. Deleting removes its effect on balances.

## Balances and settling up

Each member's balance is: the cents they paid in expenses, minus the cents they owe in `shares`, plus the cents they paid in settlements (as `from`), minus the cents they received in settlements (as `to`). A positive balance means the group owes them; negative means they owe the group. Balances in a group always sum to 0.

- `GET /api/groups/:groupId/balances` -> `200 { "balances": [ { "userId", "balanceCents" } ] }`, one entry per member in join order (including members with 0).
- `GET /api/groups/:groupId/settle-up` -> `200 { "transfers": [ { "from", "to", "amountCents" } ] }`: suggested payments that bring everybody to 0. Algorithm (so results are deterministic): repeat until all balances are 0: take the member with the most negative balance (ties: earliest to join) as the payer and the member with the most positive balance (ties: earliest to join) as the receiver, emit a transfer of `min(-payerBalance, receiverBalance)` from payer to receiver, and update both balances. Transfers are returned in the order emitted. A balanced group returns `{ "transfers": [] }`.

Recording a payment between members (a settlement):
- `POST /api/groups/:groupId/settlements` body `{ "from", "to", "amountCents" }` -> `201 { "id", "groupId", "from", "to", "amountCents", "createdBy", "createdAt" }`. `from` and `to` are different members, `amountCents` a positive integer; otherwise `400`. It changes balances as described above (`from` pays `to`).
- `GET /api/groups/:groupId/settlements` -> `200 { "settlements": [...] }`, oldest first.

## Activity feed

`GET /api/groups/:groupId/activity` -> `200 { "activity": [ { "id", "type", "actorId", "createdAt", "data" } ] }`, newest first.

| `type` | `actorId` | `data` |
|---|---|---|
| `group.created` | creator | `{ "name" }` |
| `member.added` | the member who added | `{ "userId" }` (the new member) |
| `expense.created` | `createdBy` | `{ "expenseId", "description", "amountCents" }` |
| `expense.deleted` | who deleted it | `{ "expenseId", "description", "amountCents" }` |
| `settlement.created` | `createdBy` | `{ "settlementId", "from", "to", "amountCents" }` |

Optional query `?limit=N`: an integer from 1 to 100, default 50. Anything else -> `400`.

## Browser UI

- `npm run build` writes `dist/index.html` (built by Vite). The page has `<title>Shared Expenses</title>` and a mount element `<div id="app"></div>`.
- The server serves the contents of `dist/` at the site root: `GET /` returns `dist/index.html` as `text/html`, and the script and style files it references are served too. API routes live under `/api`, so they never clash with the UI.
- The UI is a single page: sign up / log in (keep the token in `localStorage`), list and create groups, add members, add expenses (equal or exact split), show balances and the suggested settle-up transfers, and show the activity feed. Keep it small and working; it does not need to be pretty.
- Money logic for the UI lives in a plain-JS ES module `src/lib/money.js` (no imports of browser or build-tool APIs, importable straight from Node) with two named exports:

`formatCents(cents, currency = "USD")` returns a string.
- `cents` must be a safe integer, otherwise throw a `TypeError`.
- Format: optional `-`, then the currency prefix, then the amount with `,` thousands separators and exactly two decimals.
- Prefix: `USD` -> `$`, `EUR` -> `€`, `GBP` -> `£`; any other currency string `X` -> `X ` (the code and a space).
- Examples: `formatCents(0)` = `$0.00`; `formatCents(5)` = `$0.05`; `formatCents(123456)` = `$1,234.56`; `formatCents(-250, "EUR")` = `-€2.50`; `formatCents(1999, "CAD")` = `CAD 19.99`; `formatCents(-100000, "CAD")` = `-CAD 1,000.00`.

`parseAmount(str)` turns what a person typed into integer cents, or returns `null` if it is not valid. It never throws.
- Non-strings -> `null`. Leading and trailing whitespace is ignored. One optional leading `$` is allowed (no space after it).
- The rest is either plain digits, or digits with `,` thousands separators in correct groups of three (`1,234,567`), optionally followed by `.` and one or two digits. The amount must be zero or more; no sign, no exponent, no bare `.5` or `12.`.
- Compute exactly with no floating-point error. A result that is not a safe integer -> `null`.
- Examples: `"12"` -> `1200`; `"12.5"` -> `1250`; `"19.99"` -> `1999`; `"1,234.56"` -> `123456`; `"$7"` -> `700`; `" 3.10 "` -> `310`; `"0"` -> `0`; `"1.234"`, `"1,23"`, `"-5"`, `".5"`, `"12."`, `"abc"`, `""`, `"1e3"`, `"$ 5"` -> `null`.

## Robustness

- Malformed JSON, or a JSON body that is not an object, -> `400 {"error": ...}` (never a crash or an HTML error page).
- Concurrent requests must not lose or duplicate data: 25 expenses created at once are all stored, each with its own id, and balances add up.
- No response ever includes a stack trace.
