import { expect, test } from "bun:test";
import { importLedger, MAX_IMPORT_BYTES } from "./import";

test("keeps valid rows and reports each bad one", () => {
  const r = importLedger(JSON.stringify([{ kind: "deposit", cents: 5 }, { kind: "deposit", cents: -1 }, { kind: "withdraw", cents: 2 }]));
  expect(r.events).toEqual([
    { kind: "deposit", cents: 5 },
    { kind: "withdraw", cents: 2 },
  ]);
  expect(r.errors.map((e) => e.index)).toEqual([1]);
});

test("malformed JSON never throws", () => {
  expect(() => importLedger("[{")).not.toThrow();
  expect(importLedger("[{").errors[0]?.index).toBe(-1);
});

test("top level must be an array", () => {
  expect(importLedger('{"kind":"deposit","cents":5}').errors[0]?.index).toBe(-1);
});

test("rejects oversized uploads before parsing", () => {
  const big = "[" + '"x",'.repeat(MAX_IMPORT_BYTES / 4) + '"x"]';
  const r = importLedger(big);
  expect(r.events).toEqual([]);
  expect(r.errors[0]?.error).toContain("too large");
});

test("size limit counts UTF-8 bytes, not characters", () => {
  const emoji = JSON.stringify(["😀".repeat(MAX_IMPORT_BYTES / 3)]);
  expect(emoji.length).toBeLessThan(MAX_IMPORT_BYTES);
  expect(importLedger(emoji).errors[0]?.error).toContain("too large");
});

test("rejects prototype-polluting keys", () => {
  const r = importLedger('[{"kind":"deposit","cents":5,"__proto__":{"admin":true}}]');
  expect(r.events).toEqual([]);
  expect(r.errors[0]?.error).toContain("__proto__");
  expect(({} as Record<string, unknown>)["admin"]).toBeUndefined();
});
