// Compile-time contract, checked by `tsc -p .` (not run by bun test).
import { apply } from "./events";

// @ts-expect-error — "refund" is not a LedgerEvent kind.
apply(0, { kind: "refund", cents: 1 });
// @ts-expect-error — a fee must carry a reason.
apply(0, { kind: "fee", cents: 1 });
