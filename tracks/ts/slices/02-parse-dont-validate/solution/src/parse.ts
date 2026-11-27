import type { LedgerEvent } from "./events";

export type Result<T, E = string> = { ok: true; value: T } | { ok: false; error: E };

const fail = (error: string): { ok: false; error: string } => ({ ok: false, error });

/** Turn untrusted input into a LedgerEvent, or say exactly which field is wrong.
 * cents must be a positive safe integer; a fee needs a non-empty reason.
 * The error message names the offending field ("kind", "cents", "reason"). No casts. */
export function parseEvent(input: unknown): Result<LedgerEvent> {
  if (typeof input !== "object" || input === null || Array.isArray(input)) {
    return fail("event must be an object");
  }
  const kind = "kind" in input ? input.kind : undefined;
  const cents = "cents" in input ? input.cents : undefined;
  if (typeof cents !== "number" || !Number.isSafeInteger(cents) || cents <= 0) {
    return fail("cents must be a positive integer");
  }
  switch (kind) {
    case "deposit":
    case "withdraw":
      return { ok: true, value: { kind, cents } };
    case "fee": {
      const reason = "reason" in input ? input.reason : undefined;
      if (typeof reason !== "string" || reason.trim() === "") {
        return fail("fee needs a non-empty reason");
      }
      return { ok: true, value: { kind, cents, reason } };
    }
    default:
      return fail(`unknown kind: ${String(kind)}`);
  }
}
