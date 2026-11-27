import type { LedgerEvent } from "./events";
import { parseEvent } from "./parse";

export const MAX_IMPORT_BYTES = 1_000_000;

export type ImportResult = {
  events: LedgerEvent[];
  /** index is the row number, or -1 for problems with the upload as a whole. */
  errors: { index: number; error: string }[];
};

const POLLUTING = ["__proto__", "constructor", "prototype"];

/** Import a JSON array of events from an untrusted upload. Never throws.
 * Rejects uploads over MAX_IMPORT_BYTES (UTF-8) before parsing, malformed JSON, non-arrays,
 * and rows with __proto__ / constructor keys. Valid rows are kept; each bad row is reported. */
export function importLedger(json: string): ImportResult {
  const whole = (error: string): ImportResult => ({ events: [], errors: [{ index: -1, error }] });
  if (new TextEncoder().encode(json).length > MAX_IMPORT_BYTES) return whole("upload too large");
  let raw: unknown;
  try {
    raw = JSON.parse(json);
  } catch {
    return whole("malformed JSON");
  }
  if (!Array.isArray(raw)) return whole("upload must be a JSON array");
  const out: ImportResult = { events: [], errors: [] };
  raw.forEach((row: unknown, index) => {
    const bad = typeof row === "object" && row !== null && POLLUTING.find((k) => Object.hasOwn(row, k));
    if (bad) {
      out.errors.push({ index, error: `forbidden key ${bad}` });
      return;
    }
    const r = parseEvent(row);
    if (r.ok) out.events.push(r.value);
    else out.errors.push({ index, error: r.error });
  });
  return out;
}
