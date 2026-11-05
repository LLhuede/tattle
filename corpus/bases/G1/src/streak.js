// Dates are stored as local "YYYY-MM-DD" strings so a tick always belongs
// to the day the user saw on their own calendar.

export function toDateKey(date) {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, '0');
  const d = String(date.getDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}

export function addDays(dateKey, days) {
  const [y, m, d] = dateKey.split('-').map(Number);
  // Noon avoids daylight-saving edge cases when shifting days.
  return toDateKey(new Date(y, m - 1, d + days, 12));
}

/**
 * Number of consecutive completed days ending today.
 * If today isn't ticked yet, the streak still counts up to yesterday,
 * so it only breaks once a whole day has been missed.
 */
export function calculateStreak(completedDates, today = toDateKey(new Date())) {
  const done = new Set(completedDates);
  let day = done.has(today) ? today : addDays(today, -1);
  let streak = 0;
  while (done.has(day)) {
    streak++;
    day = addDays(day, -1);
  }
  return streak;
}
