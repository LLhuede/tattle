import { describe, it, expect } from 'vitest';
import { calculateStreak, addDays } from './streak.js';

const today = '2026-10-22';

describe('calculateStreak', () => {
  it('is 0 with no completions', () => {
    expect(calculateStreak([], today)).toBe(0);
  });

  it('is 1 when only today is done', () => {
    expect(calculateStreak([today], today)).toBe(1);
  });

  it('counts consecutive days ending today', () => {
    expect(calculateStreak(['2026-10-20', '2026-10-21', '2026-10-22'], today)).toBe(3);
  });

  it('keeps the streak alive when today is not ticked yet', () => {
    expect(calculateStreak(['2026-10-20', '2026-10-21'], today)).toBe(2);
  });

  it('is 0 when yesterday and today were both missed', () => {
    expect(calculateStreak(['2026-10-19', '2026-10-20'], today)).toBe(0);
  });

  it('stops at the first gap', () => {
    expect(calculateStreak(['2026-10-18', '2026-10-20', '2026-10-21', '2026-10-22'], today)).toBe(3);
  });

  it('ignores order and duplicates', () => {
    expect(calculateStreak(['2026-10-22', '2026-10-21', '2026-10-22'], today)).toBe(2);
  });

  it('works across month and year boundaries', () => {
    expect(calculateStreak(['2025-12-30', '2025-12-31', '2026-01-01'], '2026-01-01')).toBe(3);
    expect(calculateStreak(['2024-02-28', '2024-02-29', '2024-03-01'], '2024-03-01')).toBe(3);
  });
});

describe('addDays', () => {
  it('handles daylight-saving changes', () => {
    // Australian DST starts first Sunday of October; US DST ends early November.
    expect(addDays('2026-10-05', -1)).toBe('2026-10-04');
    expect(addDays('2026-11-02', -1)).toBe('2026-11-01');
  });
});
