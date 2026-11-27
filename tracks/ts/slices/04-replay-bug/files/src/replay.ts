import { apply, type LedgerEvent } from "./events";

export type Summary = { balance: number; overdrafts: number[] };

const EMPTY: Summary = { balance: 0, overdrafts: [] };

/** Fold events into a summary. overdrafts lists the indexes where the balance went below zero.
 * Calling replay must never affect another call's result. */
export function replay(events: readonly LedgerEvent[]): Summary {
  const s = EMPTY;
  events.forEach((e, i) => {
    s.balance = apply(s.balance, e);
    if (s.balance < 0) s.overdrafts.push(i);
  });
  return s;
}

/** The starting summary every replay begins from. */
export const emptySummary = (): Summary => ({ ...EMPTY, overdrafts: [...EMPTY.overdrafts] });
