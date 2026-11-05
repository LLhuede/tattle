function checkTtl(v) {
  if (v === undefined) return;
  if (typeof v !== "number" || Number.isNaN(v) || v <= 0) throw new TypeError("ttl must be a positive number");
}

export function createCache(options = {}) {
  const { maxSize = Infinity, ttl, clock = Date.now } = options;
  if (maxSize !== Infinity && !(Number.isInteger(maxSize) && maxSize > 0)) throw new TypeError("maxSize must be a positive integer or Infinity");
  checkTtl(ttl);
  if (typeof clock !== "function") throw new TypeError("clock must be a function");
  const map = new Map();
  const st = { hits: 0, misses: 0, evictions: 0, expirations: 0 };

  function purge() {
    const now = clock();
    for (const [k, e] of map) {
      if (now >= e.expiresAt) {
        map.delete(k);
        st.expirations++;
      }
    }
    return now;
  }

  const cache = {
    set(key, value, opts = {}) {
      const own = opts && opts.ttl;
      checkTtl(own);
      const d = own !== undefined ? own : ttl;
      const now = purge();
      map.delete(key);
      map.set(key, { value, expiresAt: d === undefined ? Infinity : now + d });
      while (map.size > maxSize) {
        map.delete(map.keys().next().value);
        st.evictions++;
      }
      return cache;
    },
    get(key) {
      purge();
      const e = map.get(key);
      if (!e) {
        st.misses++;
        return undefined;
      }
      st.hits++;
      map.delete(key);
      map.set(key, e);
      return e.value;
    },
    peek(key) {
      purge();
      return map.get(key)?.value;
    },
    has(key) {
      purge();
      return map.has(key);
    },
    delete(key) {
      purge();
      return map.delete(key);
    },
    clear() {
      map.clear();
    },
    get size() {
      purge();
      return map.size;
    },
    keys() {
      purge();
      return [...map.keys()];
    },
    ttlRemaining(key) {
      const now = purge();
      const e = map.get(key);
      if (!e) return undefined;
      return e.expiresAt === Infinity ? Infinity : e.expiresAt - now;
    },
    stats() {
      purge();
      return { ...st, size: map.size };
    },
    resetStats() {
      st.hits = st.misses = st.evictions = st.expirations = 0;
    },
  };
  return cache;
}

export function memoize(fn, options = {}) {
  if (typeof fn !== "function") throw new TypeError("fn must be a function");
  const { key: keyFn = (...args) => JSON.stringify(args), ...cacheOptions } = options;
  const cache = createCache(cacheOptions);
  const memoized = function (...args) {
    const k = keyFn(...args);
    const box = cache.get(k);
    if (box) return box.value;
    const v = fn.apply(this, args);
    cache.set(k, { value: v });
    if (v && typeof v.then === "function") {
      v.then(undefined, () => {
        const cur = cache.peek(k);
        if (cur && cur.value === v) cache.delete(k);
      });
    }
    return v;
  };
  memoized.cache = cache;
  memoized.clear = () => cache.clear();
  return memoized;
}
