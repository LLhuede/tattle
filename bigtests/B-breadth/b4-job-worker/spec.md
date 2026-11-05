# Job queue service

Build a small job queue service with Node.js 18 (plain JS or TypeScript, your choice): clients enqueue jobs over HTTP, and a background worker inside the same process runs them, retrying failures with exponential backoff. State survives restarts. JSON in, JSON out.

## Runtime
- `npm start` starts the server on the port in the `PORT` environment variable; the worker loop starts with it.
- Everything that must survive a restart is stored under the directory in the `DATA_DIR` environment variable (it exists and starts empty). Write nothing outside it.
- `GET /health` returns `200` with `{"ok": true}` once the server is ready.
- Errors are `{"error": "<message>"}`: `400` for invalid input (including a body that isn't valid JSON), `404` for an unknown job or route, `409` where stated below.
- Two environment variables tune timing (both integers in milliseconds):
  - `POLL_INTERVAL_MS`, default `100`: the worker looks for a job to run at least this often when idle.
  - `BACKOFF_BASE_MS`, default `1000`: the base of the retry delay (see below).
- On `SIGTERM` the process exits promptly (within about a second); it does not wait for a running job to finish.

## Jobs
A job is returned as:
```json
{ "id": "…", "type": "echo", "payload": {}, "status": "queued", "attempts": 0, "maxAttempts": 3,
  "result": null, "error": null,
  "createdAt": "<ISO 8601>", "updatedAt": "<ISO 8601>", "startedAt": null, "finishedAt": null }
```
- `id`: a string, unique and never reused (also not after a restart).
- `status`: `queued` (waiting to run, or waiting out a retry delay), `running`, `succeeded` or `failed` (gave up for good).
- `attempts`: how many attempts have started. It goes up by one, and is saved, at the moment an attempt starts.
- `result`: the handler's result once `succeeded`, otherwise `null`.
- `error`: the message of the most recent failed attempt, or `null` if no attempt has failed yet. It is kept if a later attempt succeeds.
- `startedAt`: when the most recent attempt started (`null` before the first). `finishedAt`: when the job became `succeeded` or `failed` (`null` before that). `updatedAt`: the last time anything about the job changed.

### Enqueue: `POST /jobs`
Body: `{ "type": string, "payload"?: any JSON value, "maxAttempts"?: integer }`. Responds `201` with the job as it is at creation (`status: "queued"`, `attempts: 0`). `payload` defaults to `null`. `maxAttempts` is an integer from 1 to 10 and defaults to 3. The body must be a JSON object and `type` one of the types below, otherwise `400`.

Job types (the handlers are built in):

| type | payload | behaviour |
|---|---|---|
| `echo` | anything | succeeds; `result` is the payload |
| `sum` | `{ "numbers": number[] }` (finite numbers, may be empty) | succeeds; `result` is their sum |
| `sleep` | `{ "ms": integer 0–10000 }` | waits that long, then succeeds with `result` `{ "slept": ms }` |
| `fail` | `{ "message"?: string }` or anything else | always fails; the error is `message` if it's a string, otherwise `"boom"` |
| `flaky` | `{ "failTimes": integer ≥ 0 }` | attempts number `1` to `failTimes` fail with the error `"flaky failure <n>"` (n = the attempt number); a later attempt succeeds with `result` `{ "attempts": <n> }` |

The payload for `sum`, `sleep` and `flaky` is validated when the job is enqueued (invalid gives `400`).

### Read: `GET /jobs/:id`
`200` with the job, `404` if unknown.

### List: `GET /jobs`
`200` with `{ "items": [job, …], "total": <number> }`, oldest first (order of creation). The optional query parameter `status` (one of the four statuses) keeps only jobs in that status; any other value is a `400`. `total` is the number of items.

### Stats: `GET /stats`
`200` with `{ "queued": n, "running": n, "succeeded": n, "failed": n }`; all four keys are always present.

### Retry a failed job: `POST /jobs/:id/retry`
Only for jobs with status `failed`: resets it to `queued` with `attempts` 0, `error` null, `result` null, `startedAt` null and `finishedAt` null, due immediately; responds `200` with the job. `409` if the job isn't `failed`, `404` if unknown.

## The worker
- One job runs at a time. Among the jobs that are due, the one created first runs first. A job that is waiting out a retry delay is not due, and must not hold up other jobs.
- When an attempt starts, the job becomes `running` (with `attempts` increased and `startedAt` set). When it succeeds the job becomes `succeeded`.
- When an attempt fails: if `attempts < maxAttempts`, the job goes back to `queued` and becomes due again after `BACKOFF_BASE_MS × 2^(attempts − 1)` milliseconds (so the delays after the 1st, 2nd, 3rd failure are base, 2×base, 4×base). Otherwise the job becomes `failed`.
- A new job is picked up within about `POLL_INTERVAL_MS` (plus its run time) of being enqueued.

## Persistence and restart
Every job and its state, including jobs waiting out a retry delay, must be stored so that after the server is stopped and started again with the same `DATA_DIR`:
- all jobs are still there with the same data, and ids are not reused;
- `queued` jobs are still queued (a job that was waiting out a retry delay keeps its due time) and the worker carries on with them;
- a job that was `running` when the process stopped is put back to `queued`, due immediately. The interrupted attempt still counts in `attempts`. The one exception: if that was the job's last allowed attempt (`attempts == maxAttempts`), the job becomes `failed` with `error` `"interrupted"`.
