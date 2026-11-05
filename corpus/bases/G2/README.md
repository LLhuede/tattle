# Bill splitter API

Express + SQLite (better-sqlite3) + TypeScript.

```sh
npm install
npm run dev      # http://localhost:3000 (PORT / DB_FILE env vars to override)
npm test
```

Amounts go in and out as dollars (max 2 decimal places) but are stored and calculated as integer cents.

## Endpoints

| Method | Path | Body |
| --- | --- | --- |
| GET | `/people` | |
| POST | `/people` | `{ "name": "Alice" }` |
| GET | `/expenses` | |
| GET | `/expenses/:id` | |
| POST | `/expenses` | `{ "description": "Dinner", "amount": 90, "paidBy": 1, "splitBetween": [1, 2, 3] }` |
| DELETE | `/expenses/:id` | |
| GET | `/settle-up` | |

`/settle-up` returns each person's net balance (positive = owed money) and a list of transfers:

```json
{
  "balances": [{ "personId": 1, "name": "Alice", "balance": 60 }, ...],
  "transfers": [{ "from": { "id": 2, "name": "Bob" }, "to": { "id": 1, "name": "Alice" }, "amount": 30 }, ...]
}
```

## How the maths works (`src/settle.ts`)

- Expenses are split equally. When the amount doesn't divide evenly, the spare cents go one each to the participants with the lowest ids, so shares always add up exactly to the total.
- Each person's balance = what they paid − their shares.
- Settling is greedy: the biggest debtor pays the biggest creditor, repeat. This takes at most `people − 1` transfers.
