# Stockroom: an inventory API

A small HTTP JSON API for tracking product stock, for Node.js 18. No database server: keep the data in files. Use any framework (or none).

## Running
- `npm start` starts the server on the port in the `PORT` environment variable.
- All data is stored under the directory in `DATA_DIR` (it exists and is empty on the first start). Nothing is written anywhere else. Data must survive a restart: once a response has been sent, a server started later on the same `DATA_DIR` shows the change.
- `GET /health` returns `200` with `{ "status": "ok" }` and needs no API key.
- Request and response bodies are JSON. Every error response is JSON of the form `{ "error": "<message>" }` with a 4xx status (unknown routes: 404; a body that is not valid JSON or not a JSON object: 400).

## API keys and roles
- Keys come from the `API_KEYS` environment variable: a comma-separated list of `name:key:role` entries, for example `alice:k-a1:admin,bob:k-b2:staff`. `role` is `admin` or `staff`. If `API_KEYS` is not set, no key is valid.
- Every route except `/health` needs the header `X-API-Key: <key>`.
- Checks happen in this order: key present and known (otherwise `401`), then role allowed for the route (otherwise `403`), then request validation (`400`), then the lookup (`404`), then business rules (`409`).
- `admin` may use every route. `staff` may use only: `GET /products`, `GET /products/:sku`, `GET /products/:sku/movements`, `POST /products/:sku/movements`, and `GET /reports/low-stock`.
- The **actor** of a request is the `name` of its key. Key values themselves are never stored or returned anywhere.

## Products
A product looks like this (all fields always present):
```json
{ "sku": "WID-1", "name": "Widget", "price_cents": 250, "reorder_level": 5, "stock": 0, "created_at": "2024-01-01T00:00:00.000Z", "updated_at": "2024-01-01T00:00:00.000Z" }
```
`created_at` and `updated_at` are ISO 8601 strings.

- `POST /products` (admin) creates one. Body: `sku` (required string matching `^[A-Za-z0-9._-]{1,32}$`, case-sensitive), `name` (required, non-empty after trimming, at most 100 characters), `price_cents` (integer >= 0, default 0), `reorder_level` (integer >= 0, default 0). A `stock` field in the body is ignored: new products always start with stock 0. Responds `201` with the product. Invalid input is `400`; a sku that already exists is `409`.
- `GET /products` returns `200` `{ "products": [...] }`, sorted by `sku` ascending (plain string order). Optional `q`: keep products whose sku or name contains `q`, case-insensitively.
- `GET /products/:sku` returns the product, or `404`.
- `PATCH /products/:sku` (admin) updates `name`, `price_cents` and/or `reorder_level` with the same validation as creation, and responds `200` with the updated product. The body needs at least one of those three fields; a body that contains `sku` or `stock` is `400` (they cannot be changed this way). `updated_at` is refreshed.
- `DELETE /products/:sku` (admin) responds `204` with no body. A product with stock above 0 cannot be deleted (`409`). Deleting a product also removes its movements; a product created later with the same sku starts fresh.

## Stock movements
- `POST /products/:sku/movements` (admin or staff). Body: `type` (`"in"` or `"out"`), `quantity` (integer between 1 and 1000000), `note` (optional string of at most 200 characters). `in` adds to the stock, `out` subtracts from it.
- Stock can never go below 0: an `out` larger than the current stock is `409` and changes nothing. This must hold for concurrent requests too.
- Responds `201` with the movement: `{ "id", "sku", "type", "quantity", "note", "actor", "created_at", "stock_after" }`. `note` is `null` when none was given. `id` is an integer from one counter shared by all products: the first movement ever recorded has id 1, the next 2, and so on. Ids are never reused, not even after a product is deleted.
- `GET /products/:sku/movements` returns `200` `{ "movements": [...] }` in ascending id order. Optional `type=in|out` filters (any other value is `400`).

## Low-stock report
`GET /reports/low-stock` (admin or staff) returns `200` `{ "products": [...] }`: every product with `reorder_level` above 0 whose `stock` is less than or equal to its `reorder_level`. Each entry is the product plus `shortfall` (`reorder_level - stock`, never negative). Sorted by `shortfall` descending, then `sku` ascending. Products with `reorder_level` 0 are never listed.

## Audit log
`GET /audit-log` (admin only) returns `200` `{ "entries": [...] }`, newest first (descending `id`). Optional query: `limit` (integer 1 to 200, default 50, otherwise `400`), `action` and `actor` (exact match filters).

An entry is `{ "id", "at", "actor", "action", "target", "details" }`: `id` is an integer from 1 counting every entry ever written, `at` an ISO 8601 string, `target` the sku. One entry is written for each **successful** change (never for reads, never for requests that were refused or failed):

| action | when | `details` |
|---|---|---|
| `product.create` | product created | `{}` |
| `product.update` | product patched | `{ "changed": [...] }`: the names of the fields in the PATCH body, sorted alphabetically |
| `product.delete` | product deleted | `{}` |
| `stock.in` | an `in` movement | `{ "quantity": n, "stock_after": n }` |
| `stock.out` | an `out` movement | `{ "quantity": n, "stock_after": n }` |

The audit log survives restarts like everything else.

## Tests
`npm test` runs your own tests.
