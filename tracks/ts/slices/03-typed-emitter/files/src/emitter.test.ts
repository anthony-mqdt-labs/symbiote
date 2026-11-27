import { expect, test } from "bun:test";
import { Emitter } from "./emitter";

type Events = { deposit: { cents: number }; closed: { reason: string } };

test("delivers payloads to listeners of that name only", () => {
  const bus = new Emitter<Events>();
  const got: number[] = [];
  bus.on("deposit", (p) => got.push(p.cents));
  bus.on("closed", () => got.push(-1));
  bus.emit("deposit", { cents: 5 });
  expect(got).toEqual([5]);
});

test("listeners run in subscription order", () => {
  const bus = new Emitter<Events>();
  const order: string[] = [];
  bus.on("deposit", () => order.push("a"));
  bus.on("deposit", () => order.push("b"));
  bus.emit("deposit", { cents: 1 });
  expect(order).toEqual(["a", "b"]);
});

test("unsubscribe stops delivery", () => {
  const bus = new Emitter<Events>();
  let n = 0;
  const off = bus.on("deposit", () => n++);
  bus.emit("deposit", { cents: 1 });
  off();
  bus.emit("deposit", { cents: 1 });
  expect(n).toBe(1);
});

test("unsubscribing during emit does not skip the next listener", () => {
  const bus = new Emitter<Events>();
  const seen: string[] = [];
  const offA = bus.on("deposit", () => {
    seen.push("a");
    offA();
  });
  bus.on("deposit", () => seen.push("b"));
  bus.emit("deposit", { cents: 1 });
  expect(seen).toEqual(["a", "b"]);
});

test("emit with no listeners is a no-op", () => {
  expect(() => new Emitter<Events>().emit("closed", { reason: "x" })).not.toThrow();
});
