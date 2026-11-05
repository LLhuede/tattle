// Pure settle-up maths. All money is in integer cents to avoid float rounding errors.

export interface ExpenseInput {
  amountCents: number;
  paidBy: number;
  splitBetween: number[];
}

export interface Transfer {
  from: number;
  to: number;
  amountCents: number;
}

/**
 * Split an amount as evenly as possible. Leftover cents go one each to the
 * first people in the list, so the shares always add up to the total.
 * e.g. 1000 between 3 -> [334, 333, 333]
 */
export function splitEvenly(amountCents: number, count: number): number[] {
  if (!Number.isInteger(amountCents) || amountCents < 0) {
    throw new Error("amountCents must be a non-negative integer");
  }
  if (!Number.isInteger(count) || count <= 0) {
    throw new Error("count must be a positive integer");
  }
  const base = Math.floor(amountCents / count);
  const remainder = amountCents % count;
  return Array.from({ length: count }, (_, i) => base + (i < remainder ? 1 : 0));
}

/**
 * Net balance per person: positive = they are owed money, negative = they owe.
 * Balances always sum to zero.
 */
export function computeBalances(
  personIds: number[],
  expenses: ExpenseInput[],
): Map<number, number> {
  const balances = new Map<number, number>(personIds.map((id) => [id, 0]));
  const add = (id: number, cents: number) =>
    balances.set(id, (balances.get(id) ?? 0) + cents);

  for (const expense of expenses) {
    // Sort so the "who gets the extra cent" choice doesn't depend on input order.
    const participants = [...new Set(expense.splitBetween)].sort((a, b) => a - b);
    if (participants.length === 0) {
      throw new Error("An expense must be split between at least one person");
    }
    add(expense.paidBy, expense.amountCents);
    splitEvenly(expense.amountCents, participants.length).forEach((share, i) => {
      add(participants[i], -share);
    });
  }
  return balances;
}

/**
 * Turn balances into a list of payments that settles everyone.
 * Greedy: the biggest debtor pays the biggest creditor until one is cleared.
 * This produces at most (people - 1) transfers.
 */
export function settleUp(balances: Map<number, number>): Transfer[] {
  const total = [...balances.values()].reduce((sum, b) => sum + b, 0);
  if (total !== 0) {
    throw new Error(`Balances must sum to zero (got ${total})`);
  }

  // Ties broken by id so results are deterministic.
  const byAmountThenId = (a: [number, number], b: [number, number]) =>
    b[1] - a[1] || a[0] - b[0];
  const creditors = [...balances].filter(([, b]) => b > 0).sort(byAmountThenId);
  const debtors = [...balances]
    .filter(([, b]) => b < 0)
    .map(([id, b]): [number, number] => [id, -b])
    .sort(byAmountThenId);

  const transfers: Transfer[] = [];
  let c = 0;
  let d = 0;
  while (c < creditors.length && d < debtors.length) {
    const creditor = creditors[c];
    const debtor = debtors[d];
    const amount = Math.min(creditor[1], debtor[1]);
    transfers.push({ from: debtor[0], to: creditor[0], amountCents: amount });
    creditor[1] -= amount;
    debtor[1] -= amount;
    if (creditor[1] === 0) c++;
    if (debtor[1] === 0) d++;
  }
  return transfers;
}
