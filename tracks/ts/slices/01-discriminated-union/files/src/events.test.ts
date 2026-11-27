import { expect, test } from "bun:test";
import { apply, type LedgerEvent } from "./events";

test("deposit, withdraw, fee", () => {
  expect(apply(100, { kind: "deposit", cents: 50 })).toBe(150);
  expect(apply(100, { kind: "withdraw", cents: 30 })).toBe(70);
  expect(apply(100, { kind: "fee", cents: 2, reason: "wire" })).toBe(98);
});

test("a fold over many events", () => {
  const events: LedgerEvent[] = [
    { kind: "deposit", cents: 1000 },
    { kind: "fee", cents: 25, reason: "monthly" },
    { kind: "withdraw", cents: 400 },
  ];
  expect(events.reduce(apply, 0)).toBe(575);
});

test("unknown kinds fail loudly instead of passing through", () => {
  const bogus = { kind: "refund", cents: 5 } as unknown as LedgerEvent;
  expect(() => apply(0, bogus)).toThrow(/unhandled event/);
});
