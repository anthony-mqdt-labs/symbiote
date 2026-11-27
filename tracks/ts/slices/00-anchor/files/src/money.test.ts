import { expect, test } from "bun:test";
import { formatCents } from "./money";

test("formats dollars and cents", () => {
  expect(formatCents(1234)).toBe("$12.34");
  expect(formatCents(5)).toBe("$0.05");
  expect(formatCents(0)).toBe("$0.00");
  expect(formatCents(100000)).toBe("$1000.00");
});

test("negative amounts", () => {
  expect(formatCents(-105)).toBe("-$1.05");
});

test("rejects non-integer cents", () => {
  expect(() => formatCents(12.5)).toThrow(RangeError);
  expect(() => formatCents(Number.NaN)).toThrow(RangeError);
});
