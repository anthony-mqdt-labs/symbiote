/** Format integer cents for display: 1234 → "$12.34", -105 → "-$1.05".
 * Throws RangeError for non-integers — money never touches floats. */
export function formatCents(cents: number): string {
  if (!Number.isInteger(cents)) throw new RangeError(`not integer cents: ${cents}`);
  const abs = Math.abs(cents);
  const body = `$${Math.floor(abs / 100)}.${String(abs % 100).padStart(2, "0")}`;
  return cents < 0 ? `-${body}` : body;
}
