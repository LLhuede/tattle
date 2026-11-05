# Flashcards: a spaced-repetition quiz app

A single-page flashcard app that runs in the browser, built with Vite. Users keep a deck of cards (front and back text), study the ones that are due, grade how well they remembered, and the app schedules the next review. Everything is stored in the browser; there is no server.

## Project
- Plain JavaScript (or TypeScript for the UI if you like, but see the scheduler module below). Vite is a dependency (`vite` in `devDependencies`).
- `package.json` has `"type": "module"`, a `dev` script (`vite`) and a `build` script (`vite build`). `npm run build` writes `dist/index.html`, and the built page works when the `dist` folder is served from any path or opened as a file, so asset paths in `dist/index.html` are relative (use `base: './'` in the Vite config); none starts with `/`.
- The page title is `Flashcards`, and the page contains a `<div id="app">` that the UI mounts into.
- `npm test` runs your own tests.

## The app (UI)
- A deck screen: list the cards with their due status, add a card (front and back text), edit a card, delete a card. The first time the app opens it has 5 sample cards.
- A study screen: shows the front of the next due card (the due cards are shuffled at the start of each session). The user either types an answer and the app tells them whether it matches (with `checkAnswer`), or just reveals the back. Then four buttons grade the card: Again, Hard, Good, Easy (grades 0 to 3). Grading updates the card with `review` and moves on. When no cards are due, the screen says so.
- When a session ends, a summary screen shows the points, accuracy and best streak from `sessionStats`.
- The deck, with each card's scheduling state, is saved to `localStorage` under the key `flashcards.v1` (as JSON) after every change, and loaded when the app starts.

## The scheduler module: `src/lib/scheduler.js`
All scoring and scheduling logic is in this one file. It is a plain ES module (named exports only) that imports nothing, uses no browser globals (`window`, `document`, `localStorage`) and has no side effects when imported, so it can be loaded straight into Node.js. Times are milliseconds since the epoch, always passed in (never read from a clock, except for the default `Math.random` of `shuffle`). Functions never mutate their arguments.

### Constants
- `DAY_MS = 86400000`
- `LEARN_STEP_MS = 600000` (10 minutes)

### Card state
A scheduling state is a plain object with exactly these five properties: `ease` (number), `interval` (whole days; 0 while a card is new or relearning), `reps` (consecutive successful reviews), `lapses` (times the card was forgotten) and `due` (ms timestamp).

`newCardState(now)` returns `{ ease: 2.5, interval: 0, reps: 0, lapses: 0, due: now }`.

### `review(state, grade, now)`
Returns a new state. `grade` is an integer: 0 Again, 1 Hard, 2 Good, 3 Easy; anything else (including non-integers) throws a `RangeError`. Let `ease2` be the ease after this review's adjustment:

- Ease adjustment: Again `-0.2`, Hard `-0.15`, Good `0`, Easy `+0.15`. The result is never below `1.3` and is rounded to 2 decimal places, as `Math.round(x * 100) / 100`, after each review.
- **Again (0):** `reps` becomes 0, `lapses` goes up by 1, `interval` becomes 0, and `due = now + LEARN_STEP_MS`.
- **Hard, Good, Easy (1, 2, 3):** `reps` goes up by 1, `lapses` is unchanged, and the new `interval` (whole days, at least 1) is worked out from the previous `reps` and previous `interval`:

| grade | previous reps = 0 | previous reps = 1 | previous reps >= 2 |
|---|---|---|---|
| Hard | 1 | `max(1, round(interval * 1.2))` | `max(1, round(interval * 1.2))` |
| Good | 1 | 3 | `max(1, round(interval * ease2))` |
| Easy | 4 | 7 | `max(1, round(interval * ease2 * 1.3))` |

  Here `round` is `Math.round`, and `interval * ease2 * 1.3` is evaluated left to right. Then `due = now + interval * DAY_MS`.

### `checkAnswer(given, expected)`
Returns `true` when the two strings are the same after normalising: trim leading and trailing whitespace, lower-case, and collapse every run of whitespace into one space. Nothing else is ignored (punctuation and accents count). Returns `false` if either argument is not a string, or if the normalised expected answer is empty.

### Deck helpers
A card is an object `{ id, front, back, state }` (`id` is a string). Other properties may be present and must be kept.
- `dueCards(cards, now)`: a new array with the cards whose `state.due <= now`, ordered by `state.due` ascending, ties by `id` ascending (plain string comparison with `<`). The card objects themselves are returned, not copies.
- `summarizeDeck(cards, now)`: `{ total, due, new, mature }`: the number of cards; the number due (as above); the number of new cards (`reps === 0` and `lapses === 0`); the number of mature cards (`interval >= 21`). A card can be counted in several of these.
- `shuffle(items, rng = Math.random)`: returns a new shuffled array using the Fisher-Yates algorithm exactly like this: for `i` from `length - 1` down to `1`, `j = Math.floor(rng() * (i + 1))`, swap the elements at `i` and `j`. So `rng` is called exactly `length - 1` times (not at all for 0 or 1 items).

### `sessionStats(grades)`
`grades` is an array of grades (0 to 3, same rule as `review`: any other value throws a `RangeError`). Returns `{ total, correct, accuracy, points, bestStreak }`:
- `total`: the number of grades. `correct`: how many are 2 or 3.
- `accuracy`: `correct / total` rounded to 2 decimals (`Math.round(x * 100) / 100`), or `0` when `total` is 0.
- A **streak** is a run of consecutive grades that are 2 or 3; grades 0 and 1 reset the streak to 0. `bestStreak` is the longest streak.
- `points`: the sum over all grades of `base[grade] + bonus`, where `base` is `[0, 5, 10, 15]` for grades 0, 1, 2, 3, and `bonus` is `2 * min(streak - 1, 5)` for a grade of 2 or 3 (with `streak` counting the current answer, so the first answer of a streak gets no bonus), and 0 for a grade of 0 or 1.
- For an empty array everything is 0.
