const PREFIX = { USD: "$", EUR: "€", GBP: "£" };

export function formatCents(cents, currency = "USD") {
  if (!Number.isSafeInteger(cents)) throw new TypeError("cents must be a safe integer");
  if (typeof currency !== "string") throw new TypeError("currency must be a string");
  const abs = Math.abs(cents);
  const whole = String(Math.floor(abs / 100)).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const frac = String(abs % 100).padStart(2, "0");
  const prefix = PREFIX[currency] ?? `${currency} `;
  return `${cents < 0 ? "-" : ""}${prefix}${whole}.${frac}`;
}

export function parseAmount(str) {
  if (typeof str !== "string") return null;
  let s = str.trim();
  if (s.startsWith("$")) s = s.slice(1);
  const m = /^(\d+|\d{1,3}(?:,\d{3})+)(?:\.(\d{1,2}))?$/.exec(s);
  if (!m) return null;
  const whole = Number(m[1].replace(/,/g, ""));
  const frac = Number((m[2] ?? "").padEnd(2, "0"));
  const cents = whole * 100 + frac;
  return Number.isSafeInteger(cents) ? cents : null;
}
