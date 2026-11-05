import { describe, expect, it } from "vitest";
import { computeBalances, ExpenseInput, settleUp, splitEvenly, Transfer } from "../src/settle";

const ALICE = 1;
const BOB = 2;
const CAROL = 3;
const DAVE = 4;

const sum = (xs: number[]) => xs.reduce((a, b) => a + b, 0);

/** Apply transfers to balances and check everyone ends at zero. */
function applyTransfers(balances: Map<number, number>, transfers: Transfer[]) {
  const after = new Map(balances);
  for (const t of transfers) {
    after.set(t.from, after.get(t.from)! + t.amountCents);
    after.set(t.to, after.get(t.to)! - t.amountCents);
  }
  return after;
}

describe("splitEvenly", () => {
  it("splits an evenly divisible amount", () => {
    expect(splitEvenly(900, 3)).toEqual([300, 300, 300]);
  });

  it("gives leftover cents to the first people so shares add up to the total", () => {
    expect(splitEvenly(1000, 3)).toEqual([334, 333, 333]);
    expect(splitEvenly(1001, 3)).toEqual([334, 334, 333]);
  });

  it("handles amounts smaller than the number of people", () => {
    expect(splitEvenly(2, 4)).toEqual([1, 1, 0, 0]);
  });

  it("handles a single person", () => {
    expect(splitEvenly(1234, 1)).toEqual([1234]);
  });

  it("always sums back to the original amount", () => {
    for (let amount = 0; amount < 500; amount += 7) {
      for (let n = 1; n <= 9; n++) {
        expect(sum(splitEvenly(amount, n))).toBe(amount);
      }
    }
  });

  it("rejects bad input", () => {
    expect(() => splitEvenly(100, 0)).toThrow();
    expect(() => splitEvenly(-1, 2)).toThrow();
    expect(() => splitEvenly(10.5, 2)).toThrow();
  });
});

describe("computeBalances", () => {
  it("is all zeros with no expenses", () => {
    const balances = computeBalances([ALICE, BOB], []);
    expect([...balances]).toEqual([
      [ALICE, 0],
      [BOB, 0],
    ]);
  });

  it("credits the payer and debits each participant", () => {
    // Alice pays $30 dinner for Alice, Bob, Carol
    const balances = computeBalances(
      [ALICE, BOB, CAROL],
      [{ amountCents: 3000, paidBy: ALICE, splitBetween: [ALICE, BOB, CAROL] }],
    );
    expect(balances.get(ALICE)).toBe(2000);
    expect(balances.get(BOB)).toBe(-1000);
    expect(balances.get(CAROL)).toBe(-1000);
  });

  it("handles a payer who is not part of the split", () => {
    // Alice buys Bob and Carol a $20 gift card each
    const balances = computeBalances(
      [ALICE, BOB, CAROL],
      [{ amountCents: 4000, paidBy: ALICE, splitBetween: [BOB, CAROL] }],
    );
    expect(balances.get(ALICE)).toBe(4000);
    expect(balances.get(BOB)).toBe(-2000);
    expect(balances.get(CAROL)).toBe(-2000);
  });

  it("is zero when someone pays only for themselves", () => {
    const balances = computeBalances(
      [ALICE, BOB],
      [{ amountCents: 999, paidBy: BOB, splitBetween: [BOB] }],
    );
    expect(balances.get(ALICE)).toBe(0);
    expect(balances.get(BOB)).toBe(0);
  });

  it("nets out multiple expenses", () => {
    const balances = computeBalances(
      [ALICE, BOB],
      [
        { amountCents: 1000, paidBy: ALICE, splitBetween: [ALICE, BOB] },
        { amountCents: 600, paidBy: BOB, splitBetween: [ALICE, BOB] },
      ],
    );
    expect(balances.get(ALICE)).toBe(200);
    expect(balances.get(BOB)).toBe(-200);
  });

  it("ignores duplicate ids in splitBetween", () => {
    const balances = computeBalances(
      [ALICE, BOB],
      [{ amountCents: 1000, paidBy: ALICE, splitBetween: [BOB, BOB, ALICE] }],
    );
    expect(balances.get(ALICE)).toBe(500);
    expect(balances.get(BOB)).toBe(-500);
  });

  it("assigns the extra cent the same way regardless of input order", () => {
    const a = computeBalances([ALICE, BOB, CAROL], [
      { amountCents: 1000, paidBy: ALICE, splitBetween: [CAROL, BOB, ALICE] },
    ]);
    const b = computeBalances([ALICE, BOB, CAROL], [
      { amountCents: 1000, paidBy: ALICE, splitBetween: [ALICE, BOB, CAROL] },
    ]);
    expect([...a]).toEqual([...b]);
  });

  it("always sums to zero, even with uneven cents", () => {
    const balances = computeBalances(
      [ALICE, BOB, CAROL, DAVE],
      [
        { amountCents: 1000, paidBy: ALICE, splitBetween: [ALICE, BOB, CAROL] },
        { amountCents: 777, paidBy: BOB, splitBetween: [ALICE, BOB, CAROL, DAVE] },
        { amountCents: 1, paidBy: DAVE, splitBetween: [ALICE, CAROL] },
      ],
    );
    expect(sum([...balances.values()])).toBe(0);
  });

  it("throws on an expense with no participants", () => {
    expect(() =>
      computeBalances([ALICE], [{ amountCents: 100, paidBy: ALICE, splitBetween: [] }]),
    ).toThrow();
  });
});

describe("settleUp", () => {
  it("returns no transfers when everyone is square", () => {
    expect(settleUp(new Map([[ALICE, 0], [BOB, 0]]))).toEqual([]);
    expect(settleUp(new Map())).toEqual([]);
  });

  it("makes a single transfer between two people", () => {
    expect(settleUp(new Map([[ALICE, 500], [BOB, -500]]))).toEqual([
      { from: BOB, to: ALICE, amountCents: 500 },
    ]);
  });

  it("has several debtors pay one creditor", () => {
    expect(settleUp(new Map([[ALICE, 2000], [BOB, -1000], [CAROL, -1000]]))).toEqual([
      { from: BOB, to: ALICE, amountCents: 1000 },
      { from: CAROL, to: ALICE, amountCents: 1000 },
    ]);
  });

  it("has one debtor pay several creditors", () => {
    expect(settleUp(new Map([[ALICE, 300], [BOB, 700], [CAROL, -1000]]))).toEqual([
      { from: CAROL, to: BOB, amountCents: 700 },
      { from: CAROL, to: ALICE, amountCents: 300 },
    ]);
  });

  it("matches the biggest debtor with the biggest creditor first", () => {
    const transfers = settleUp(
      new Map([[ALICE, 1500], [BOB, 500], [CAROL, -1500], [DAVE, -500]]),
    );
    expect(transfers).toEqual([
      { from: CAROL, to: ALICE, amountCents: 1500 },
      { from: DAVE, to: BOB, amountCents: 500 },
    ]);
  });

  it("never produces zero-value or negative transfers", () => {
    const transfers = settleUp(
      new Map([[ALICE, 1000], [BOB, 0], [CAROL, -400], [DAVE, -600]]),
    );
    for (const t of transfers) expect(t.amountCents).toBeGreaterThan(0);
    expect(transfers.some((t) => t.from === BOB || t.to === BOB)).toBe(false);
  });

  it("refuses balances that don't sum to zero", () => {
    expect(() => settleUp(new Map([[ALICE, 100], [BOB, -99]]))).toThrow(/sum to zero/);
  });

  it("works end to end for a weekend away", () => {
    // Alice pays $120 accommodation for all 4
    // Bob pays $45.50 groceries for all 4
    // Carol pays $30 taxi for Carol and Dave
    // Dave pays $10 coffee for Alice and Bob
    const expenses: ExpenseInput[] = [
      { amountCents: 12000, paidBy: ALICE, splitBetween: [ALICE, BOB, CAROL, DAVE] },
      { amountCents: 4550, paidBy: BOB, splitBetween: [ALICE, BOB, CAROL, DAVE] },
      { amountCents: 3000, paidBy: CAROL, splitBetween: [CAROL, DAVE] },
      { amountCents: 1000, paidBy: DAVE, splitBetween: [ALICE, BOB] },
    ];
    const balances = computeBalances([ALICE, BOB, CAROL, DAVE], expenses);

    // Shares: accom 3000 each, groceries 1138/1138/1137/1137, taxi 1500 each, coffee 500 each
    expect(balances.get(ALICE)).toBe(12000 - 3000 - 1138 - 500); // 7362
    expect(balances.get(BOB)).toBe(4550 - 3000 - 1138 - 500); // -88
    expect(balances.get(CAROL)).toBe(3000 - 3000 - 1137 - 1500); // -2637
    expect(balances.get(DAVE)).toBe(1000 - 3000 - 1137 - 1500); // -4637

    const transfers = settleUp(balances);
    expect(transfers).toEqual([
      { from: DAVE, to: ALICE, amountCents: 4637 },
      { from: CAROL, to: ALICE, amountCents: 2637 },
      { from: BOB, to: ALICE, amountCents: 88 },
    ]);
    expect([...applyTransfers(balances, transfers).values()].every((b) => b === 0)).toBe(true);
  });

  it("settles everyone to zero in at most n-1 transfers (randomised)", () => {
    // Small seeded PRNG so failures are reproducible
    let seed = 42;
    const rand = () => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) / 2 ** 31);
    const randInt = (max: number) => Math.floor(rand() * max);

    for (let run = 0; run < 200; run++) {
      const people = Array.from({ length: 2 + randInt(8) }, (_, i) => i + 1);
      const expenses: ExpenseInput[] = Array.from({ length: randInt(15) }, () => {
        const splitBetween = people.filter(() => rand() < 0.6);
        return {
          amountCents: 1 + randInt(50000),
          paidBy: people[randInt(people.length)],
          splitBetween: splitBetween.length ? splitBetween : [people[0]],
        };
      });

      const balances = computeBalances(people, expenses);
      expect(sum([...balances.values()])).toBe(0);

      const transfers = settleUp(balances);
      expect(transfers.length).toBeLessThanOrEqual(people.length - 1);
      for (const t of transfers) {
        expect(Number.isInteger(t.amountCents)).toBe(true);
        expect(t.amountCents).toBeGreaterThan(0);
        expect(t.from).not.toBe(t.to);
      }
      expect([...applyTransfers(balances, transfers).values()].every((b) => b === 0)).toBe(true);
    }
  });
});
