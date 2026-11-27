import type { LedgerEvent } from "./events";

export type Result<T, E = string> = { ok: true; value: T } | { ok: false; error: E };

/** Turn untrusted input into a LedgerEvent, or say exactly which field is wrong.
 * cents must be a positive safe integer; a fee needs a non-empty reason.
 * The error message names the offending field ("kind", "cents", "reason"). No casts. */
export function parseEvent(input: unknown): Result<LedgerEvent> {
  // TODO
  return { ok: false, error: "not implemented" };
}
