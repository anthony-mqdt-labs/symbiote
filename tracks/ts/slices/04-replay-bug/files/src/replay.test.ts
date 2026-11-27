import { expect, test } from "bun:test";
import type { LedgerEvent } from "./events";
import { replay } from "./replay";

const day1: LedgerEvent[] = [
  { kind: "deposit", cents: 100 },
  { kind: "withdraw", cents: 150 },
  { kind: "deposit", cents: 100 },
];

test("summarizes balance and overdraft indexes", () => {
  expect(replay(day1)).toEqual({ balance: 50, overdrafts: [1] });
});

test("calls are independent", () => {
  const a = replay(day1);
  const b = replay(day1);
  expect(b).toEqual({ balance: 50, overdrafts: [1] });
  expect(a).not.toBe(b);
  expect(replay([])).toEqual({ balance: 0, overdrafts: [] });
});
