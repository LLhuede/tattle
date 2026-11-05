export const DAY_MS = 86400000;
export const LEARN_STEP_MS = 600000;

const round2 = (x) => Math.round(x * 100) / 100;

function checkGrade(g) {
  if (!Number.isInteger(g) || g < 0 || g > 3) throw new RangeError(`grade must be an integer 0-3, got ${g}`);
}

export function newCardState(now) {
  return { ease: 2.5, interval: 0, reps: 0, lapses: 0, due: now };
}

export function review(state, grade, now) {
  checkGrade(grade);
  const ease2 = Math.max(1.3, round2(state.ease + [-0.2, -0.15, 0, 0.15][grade]));
  if (grade === 0) {
    return { ease: ease2, interval: 0, reps: 0, lapses: state.lapses + 1, due: now + LEARN_STEP_MS };
  }
  const { reps, interval } = state;
  let next;
  if (grade === 1) next = reps === 0 ? 1 : Math.max(1, Math.round(interval * 1.2));
  else if (grade === 2) next = reps === 0 ? 1 : reps === 1 ? 3 : Math.max(1, Math.round(interval * ease2));
  else next = reps === 0 ? 4 : reps === 1 ? 7 : Math.max(1, Math.round(interval * ease2 * 1.3));
  return { ease: ease2, interval: next, reps: reps + 1, lapses: state.lapses, due: now + next * DAY_MS };
}

const norm = (s) => s.trim().toLowerCase().replace(/\s+/g, " ");

export function checkAnswer(given, expected) {
  if (typeof given !== "string" || typeof expected !== "string") return false;
  const e = norm(expected);
  return e !== "" && norm(given) === e;
}

export function dueCards(cards, now) {
  return cards
    .filter((c) => c.state.due <= now)
    .sort((a, b) => a.state.due - b.state.due || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

export function summarizeDeck(cards, now) {
  return {
    total: cards.length,
    due: cards.filter((c) => c.state.due <= now).length,
    new: cards.filter((c) => c.state.reps === 0 && c.state.lapses === 0).length,
    mature: cards.filter((c) => c.state.interval >= 21).length,
  };
}

export function shuffle(items, rng = Math.random) {
  const a = [...items];
  for (let i = a.length - 1; i >= 1; i--) {
    const j = Math.floor(rng() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

export function sessionStats(grades) {
  for (const g of grades) checkGrade(g);
  const base = [0, 5, 10, 15];
  let streak = 0;
  let best = 0;
  let points = 0;
  let correct = 0;
  for (const g of grades) {
    if (g >= 2) {
      streak++;
      correct++;
      best = Math.max(best, streak);
      points += base[g] + 2 * Math.min(streak - 1, 5);
    } else {
      streak = 0;
      points += base[g];
    }
  }
  const total = grades.length;
  return { total, correct, accuracy: total ? Math.round((correct / total) * 100) / 100 : 0, points, bestStreak: best };
}
