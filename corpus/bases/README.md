# Corpus bases

Frozen starting apps for edit prompts. Every run of an edit prompt starts
from exactly this code (`cv sandbox init <dir> --base corpus/bases/<name>`).

| Base | Made from | Used by | Notes |
|---|---|---|---|
| `G1` | `fixtures/recorded/G1/run1` (final snapshot) | E2, V1, T2, T3 | React + Vite habit tracker. The agent chose JavaScript. |
| `G2` | `fixtures/recorded/G2/run1` (final snapshot) | E1, T4, T5 (E4 once seeded) | TypeScript + Express bill splitter, 27 passing tests |
| `G3` | `fixtures/recorded/G3/run1` (final snapshot) | E3, V2 | TypeScript + Express URL shortener, 30 passing tests |
| `T1` | `G3` + seeded bug | T1 | `src/code.ts`: `CODE_LENGTH` changed from 6 to 8. The existing test (correctly) expects 6. |

Made with `cv record restore`. `node_modules` is never included; the agent
installs dependencies itself.

## Recall-study bases (seeded traps)

| Base | Made from | Seed |
|---|---|---|
| `G2F` | `G2` | `tests/rounding.test.ts` expects spare cents to go to the *last* participants; the code gives them to the lowest ids, so this test fails from the start |
| `G3B` | `G3` | `npm test` points at a missing `vitest.ci.config.ts`, so the test command errors before running any test |
| `G2S` | `G2` | `README.md` documents `GET /people/:id/balance`, which does not exist in the code |

## Recall-study held-out batch 3 bases (real, untested bugs)

Each is a copy of a clean base with one small genuine bug. All existing tests still pass.

| Base | Made from | Bug |
|---|---|---|
| `G2X` | `G2` | `toCents` in `src/app.ts` checks `amount < 0` instead of `<= 0`, so `POST /expenses` with `amount: 0` passes validation and hits the DB `CHECK (amount_cents > 0)`, returning 500 instead of 400. No existing test posts a zero amount. |
| `G3X` | `G3` | `isHttpUrl` in `src/app.ts` uses `value.length >= MAX_URL_LENGTH`, so a URL of exactly 2048 characters is rejected (README: only longer than 2048). The "too long" test uses 3000 chars. |
| `G3Y` | `G3` | `ALPHABET` in `src/code.ts` is missing lowercase `z` (61 characters), so codes never contain `z`. README says `[A-Za-z0-9]`; tests only check the regex, which still passes. |

Latent (unseeded) real defects in the clean bases, used by held-out batch 3: `G3` does not trim a trailing slash from `BASE_URL` (links come back with `//`); `G2` treats person names as case-sensitive (`Alice` and `alice` both accepted).
