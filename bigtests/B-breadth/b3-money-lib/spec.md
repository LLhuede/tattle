# money: a currency-safe money library

Build a small JavaScript library for working with money, for Node.js 18 (plain JS or TypeScript, your choice, no runtime dependencies needed). It is imported through the package's `main` (or `exports["."]`) entry, which must be loadable by Node after `npm run build` if you have a build step; CommonJS or ES modules are both fine. Everything below is a **named export** of that entry.

## Representation
- A money value is a plain object with exactly two own properties: `{ amount, currency }`.
  - `amount` is an integer number of **minor units** (cents for USD). It is always a safe integer (`Number.isSafeInteger`) and is never `-0`.
  - `currency` is a currency code such as `"USD"`.
- Functions never mutate their arguments; they return new values.
- Every function that is told to "throw" throws an `Error` (any subclass is fine). Money values passed to functions are assumed to be well formed, but a function that takes a currency code throws for one that isn't registered.

## Currency registry
A currency is `{ code, symbol, exponent, name }`; `exponent` is the number of decimal digits (USD has 2, JPY has 0, KWD has 3). The registry starts with:

| code | symbol | exponent | name |
|---|---|---|---|
| USD | `$` | 2 | US Dollar |
| EUR | `€` | 2 | Euro |
| GBP | `£` | 2 | Pound Sterling |
| JPY | `¥` | 0 | Yen |
| CHF | `CHF` | 2 | Swiss Franc |
| KWD | `KD` | 3 | Kuwaiti Dinar |

- `getCurrency(code)`: the currency object for that exact (upper-case) code, or `undefined` if it isn't registered.
- `listCurrencies()`: an array of all registered currency objects, sorted by `code`.
- `registerCurrency({ code, symbol, exponent, name? })`: adds a currency and returns it. `code` must be exactly three upper-case ASCII letters, `symbol` a non-empty string, `exponent` an integer from 0 to 6. `name` defaults to the code. Throws for anything invalid or if the code is already registered.

## Construction and arithmetic
- `money(amount, currency)`: returns `{ amount, currency }`. Throws if `amount` isn't a safe integer or the currency isn't registered. `money(-0, "USD")` gives an amount of `0`.
- `add(a, b)` and `subtract(a, b)`: same currency required (throw otherwise). Throw if the result isn't a safe integer.
- `negate(m)`: the same currency with the sign flipped (`negate(money(0, "USD")).amount` is `0`, not `-0`).
- `compare(a, b)`: `-1`, `0` or `1`; throws if the currencies differ.
- `equals(a, b)`: `true` when amount and currency are both equal. Different currencies give `false` (no throw).

## Multiplication and allocation
- `multiply(m, factor)`: `factor` is a finite number (throw otherwise). The result is the exact product `amount × factor`, rounded to a whole number of minor units with **half away from zero** (`2.5` becomes `3`, `-2.5` becomes `-3`). "Exact" means the factor is taken as the decimal it prints as (`String(factor)`), so `multiply(money(100, "USD"), 1.005)` is exactly `100.5` and rounds to `101`, even though `100 * 1.005` is `100.49999999999999` in floating point. Throws if the result isn't a safe integer.
- `allocate(m, ratios)`: splits `m` into `ratios.length` parts proportionally, without losing or creating any minor unit. `ratios` is a non-empty array of non-negative safe integers whose sum is greater than 0 (throw otherwise). Algorithm:
  1. Work with the absolute value of the amount (a negative amount is allocated as a positive one and every part is then negated).
  2. Part `i` starts as `floor(amount × ratios[i] / total)`. Compute this exactly, even when `amount × ratios[i]` is larger than 2^53.
  3. The leftover minor units (always fewer than the number of non-zero ratios) are handed out one at a time, in order from the first part to the last, skipping parts whose ratio is 0. A part with ratio 0 is always 0.
  Returns an array of money values in the same currency and order; they always add up to exactly `m`.
- `split(m, n)`: equal split into `n` parts, which is `allocate(m, [1, 1, …])` with `n` ones. `n` must be a positive safe integer (throw otherwise).

## Formatting
`format(m, options?)` returns a string. The currency's `exponent` fixes how many decimal digits are shown (none, and no decimal point, for exponent 0).
- The integer part is grouped in thousands with `,` and the decimal separator is `.`. Digits come from the integer `amount` itself (not from floating-point division), so it is exact for every safe integer.
- A negative amount puts `-` first, before the symbol: `-$1,234.56`. Zero has no sign.
- `options.style` is `"symbol"` (default): the currency symbol directly in front of the number with no space (`$1,234.56`, `¥500`, `CHF1,234.50`, `KD1.234`); `"code"`: the number then a space and the code (`1,234.56 USD`, `-5.00 EUR`); or `"none"`: just the number (`-1,234.56`). Any other style throws.

## Parsing
`parse(text, currency?)` returns a money value from text, or throws. `currency`, if given, is a currency code.
- Surrounding whitespace is ignored. The text may start with `-` for a negative amount.
- The number is digits with optional `,` thousands separators (either no separators, like `1234`, or groups of exactly three digits after the first group, like `1,234,567`), optionally followed by `.` and one or more digits. Forms like `.5` or `5.` are invalid.
- The currency comes from one of these forms (after the optional `-`): a symbol directly before the number (`$12.50`), a registered code after the number separated by spaces (`12.50 EUR`), a registered code before the number separated by spaces (`EUR 12.50`), or no marker at all (`12.50`).
- If the text has a symbol, it must match a registered currency's symbol. If more than one registered currency uses the same symbol, the symbol alone is ambiguous: it is accepted only when the `currency` argument is one of those currencies.
- If the text names a currency (by symbol or code) and the `currency` argument is also given, they must agree (throw if not). If neither names a currency, throw.
- The number of decimal digits must not exceed the currency's exponent (never round: `"1.234"` for USD throws); fewer is fine (`"5"` is 500 minor units for USD, `"5.5"` is 550). The result must be a safe integer, otherwise throw.
- `parse("-0")`-style input gives an amount of `0`.
- `parse(format(m))` gives back `m` for any value in a currency whose symbol is unique.

## Major units
- `fromMajor(value, currency)`: converts a number of major units (for example `12.34` dollars) to a money value. `value` must be a finite number. It is rounded half away from zero to the currency's exponent, using the decimal the number prints as (`String(value)`), so `fromMajor(1.005, "USD")` is `101` minor units, not `100`. Throws for an unregistered currency or a result that isn't a safe integer.
- `toMajor(m)`: the amount in major units as a number: `amount / 10^exponent` (so `toMajor(money(1234, "USD"))` is `12.34`).
