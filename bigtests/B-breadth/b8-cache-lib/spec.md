# cachekit: a TTL + LRU cache library

A small in-memory cache library for Node.js 18, with no runtime dependencies.

## Package
- `package.json` has `"name": "cachekit"` and a `main` field pointing at the entry file (suggested: `src/index.js`). Plain JavaScript (CommonJS or ES modules) is fine, and so is TypeScript if `npm run build` compiles it and `main` points at the built JS.
- The entry exports exactly these two functions by name: `createCache` and `memoize`.
- `npm test` runs your own tests.

## `createCache(options?)`
Returns a cache object. `options` (all optional):
- `maxSize`: the most live entries the cache holds. A positive integer, or `Infinity` (the default, meaning no limit). Anything else throws a `TypeError`.
- `ttl`: the default time-to-live of an entry, in milliseconds. A positive number (`Infinity` allowed). Default: no expiry. Zero, negative, `NaN` or a non-number throws a `TypeError`.
- `clock`: a function returning the current time in milliseconds. Default `Date.now`. Every time reading the cache does goes through this function, so tests can inject a fake clock.

### Methods and properties
Keys can be any value and are compared like `Map` keys (so `1` and `"1"` are different keys). Values can be anything, including `undefined`.

- `set(key, value, { ttl }?)`: stores the entry and returns the cache itself (so calls can be chained). The optional per-key `ttl` has the same rules as the option and overrides the default; `ttl: Infinity` means this key never expires even if the cache has a default ttl. An invalid per-key `ttl` throws a `TypeError` and changes nothing. Setting an existing key replaces its value and its expiry (the expiry is measured again from now), and makes it the most recently used. A new key that would take the cache above `maxSize` evicts the least recently used entries until it fits.
- `get(key)`: returns the value, or `undefined` if the key is absent or expired. A hit makes the entry the most recently used. A get does not extend the entry's expiry.
- `peek(key)`: like `get` but changes no recency and is not counted in the stats.
- `has(key)`: `true` if the key is present and not expired. Changes no recency and is not counted in the stats.
- `delete(key)`: removes the entry; returns `true` if a live entry was removed, otherwise `false`.
- `clear()`: removes every entry. Returns nothing.
- `size`: a property (not a method) with the number of live entries.
- `keys()`: an array of the live keys, from least recently used to most recently used.
- `ttlRemaining(key)`: the milliseconds left before the entry expires; `Infinity` if it never expires; `undefined` if the key is absent or expired.
- `stats()`: a fresh plain object with exactly these five properties: `hits`, `misses`, `evictions`, `expirations`, `size`. See below.
- `resetStats()`: sets the four counters to zero. It does not touch the entries.

### Expiry
- An entry set at time `t` with ttl `d` expires at time `t + d`. It is expired whenever `clock() >= t + d` (so at exactly `d` ms it is already gone).
- Every operation (including `size`, `keys()`, `stats()` and `ttlRemaining`) first removes all expired entries. Each removed entry adds 1 to `expirations`. Expired entries therefore never count against `maxSize`, and no entry is counted as expired twice.

### Stats
- `hits`: number of `get` calls that returned a live entry. `misses`: number of `get` calls that did not (absent or expired).
- `evictions`: entries removed because the cache was over `maxSize`. Entries removed by `delete`, `clear`, overwriting, or expiry are not evictions.
- `expirations`: see Expiry. `size`: same as the `size` property.
- `resetStats()` zeroes `hits`, `misses`, `evictions` and `expirations`.

## `memoize(fn, options?)`
Returns a function that behaves like `fn` but caches results.
- `fn` must be a function, otherwise `TypeError`.
- `options` may hold `maxSize`, `ttl` and `clock` (same meaning as for `createCache`; they configure an internal cache) and `key`: a function called with the same arguments as `fn`, returning the cache key. The default key is `JSON.stringify(args)`, where `args` is the array of arguments.
- Each call looks the key up in the cache once, so each call counts as exactly one hit or one miss in the cache's stats. On a miss, `fn` is called (with the same `this` and arguments) and its result is stored. Results that are `undefined` are cached too.
- If `fn` throws, the error propagates and nothing is cached.
- If `fn` returns a promise, the promise itself is cached, so concurrent callers share one call to `fn`. If that promise rejects, the entry is removed, so a call made after you have seen the rejection runs `fn` again.
- The returned function has two extra properties: `cache` (the internal cache, so `memoized.cache.stats()` works) and `clear()` (removes all cached results). The shape of the values stored inside `memoized.cache` is not specified; only its `stats()`, `size` and `keys().length` are meant to be used from outside.
