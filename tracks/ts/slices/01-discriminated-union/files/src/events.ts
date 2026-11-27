export type LedgerEvent =
  | { kind: "deposit"; cents: number }
  | { kind: "withdraw"; cents: number }
  | { kind: "fee"; cents: number; reason: string };

/** Exhaustiveness guard: only type-checks when every variant was handled. */
export function assertNever(x: never): never {
  throw new Error(`unhandled event: ${JSON.stringify(x)}`);
}

/** The balance after applying e. Deposits add; withdrawals and fees subtract. */
export function apply(balance: number, e: LedgerEvent): number {
  // TODO: switch (e.kind) { case ...: ... default: return assertNever(e) }
  return balance;
}
