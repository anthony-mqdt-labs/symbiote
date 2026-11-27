import { expect, test } from "bun:test";
import { parseEvent } from "./parse";

test("accepts every valid variant", () => {
  expect(parseEvent({ kind: "deposit", cents: 500 })).toEqual({ ok: true, value: { kind: "deposit", cents: 500 } });
  expect(parseEvent({ kind: "withdraw", cents: 1 })).toEqual({ ok: true, value: { kind: "withdraw", cents: 1 } });
  expect(parseEvent({ kind: "fee", cents: 3, reason: "wire" })).toEqual({
    ok: true,
    value: { kind: "fee", cents: 3, reason: "wire" },
  });
});

test("drops unknown extra fields from the parsed value", () => {
  const r = parseEvent({ kind: "deposit", cents: 5, admin: true });
  expect(r).toEqual({ ok: true, value: { kind: "deposit", cents: 5 } });
});

const rejects: [string, unknown, string][] = [
  ["null", null, "object"],
  ["array", [1, 2], "object"],
  ["string", "deposit", "object"],
  ["unknown kind", { kind: "refund", cents: 5 }, "kind"],
  ["missing kind", { cents: 5 }, "kind"],
  ["string cents", { kind: "deposit", cents: "5" }, "cents"],
  ["negative", { kind: "deposit", cents: -5 }, "cents"],
  ["zero", { kind: "withdraw", cents: 0 }, "cents"],
  ["fractional", { kind: "deposit", cents: 1.5 }, "cents"],
  ["NaN", { kind: "deposit", cents: Number.NaN }, "cents"],
  ["fee without reason", { kind: "fee", cents: 2 }, "reason"],
  ["fee blank reason", { kind: "fee", cents: 2, reason: "  " }, "reason"],
];

for (const [name, input, field] of rejects) {
  test(`rejects ${name}`, () => {
    const r = parseEvent(input);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.error).toContain(field);
  });
}
