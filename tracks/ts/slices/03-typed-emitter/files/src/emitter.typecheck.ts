// Compile-time contract, checked by `tsc -p .` (not run by bun test).
import { Emitter } from "./emitter";

type Events = { deposit: { cents: number }; closed: { reason: string } };
const bus = new Emitter<Events>();

bus.on("deposit", (p) => {
  const cents: number = p.cents;
  void cents;
});
// @ts-expect-error — payload must match the event name.
bus.emit("deposit", { reason: "x" });
// @ts-expect-error — unknown event name.
bus.on("nope", () => {});
