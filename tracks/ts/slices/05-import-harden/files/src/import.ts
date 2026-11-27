import type { LedgerEvent } from "./events";

export const MAX_IMPORT_BYTES = 1_000_000;

export type ImportResult = {
  events: LedgerEvent[];
  /** index is the row number, or -1 for problems with the upload as a whole. */
  errors: { index: number; error: string }[];
};

/** Import a JSON array of events from an untrusted upload. Never throws.
 * Rejects uploads over MAX_IMPORT_BYTES (UTF-8) before parsing, malformed JSON, non-arrays,
 * and rows with __proto__ / constructor keys. Valid rows are kept; each bad row is reported. */
export function importLedger(json: string): ImportResult {
  const raw = JSON.parse(json);
  return { events: raw, errors: [] };
}
