/**
 * Encode a request body that may carry `bigint`.
 *
 * **`JSON.stringify` throws on a `bigint`** — *"cannot serialize BigInt"*, a `TypeError`, not a
 * silent drop. That is not a hypothetical: ts-rs maps Rust's `u64` to TypeScript's `bigint`, so a
 * generated wire type carries them wherever the Rust side counts something. `Composition.terms` is
 * `{ [key in BoundTerm]?: bigint }` (`types/generated/query.ts:147`), which means **a composition
 * built faithfully against its own generated type cannot be sent by a bare `JSON.stringify`.**
 *
 * The failure is invisible to unit tests over the builder, because comparing objects never
 * serializes them. It surfaces only on the first real request.
 *
 * A `bigint` is encoded as a **JSON number**, which is what the Rust side reads it back as: serde
 * deserializes `u64` from a JSON number, and JSON has no wider integer to offer. But JSON numbers
 * are IEEE-754 doubles, so above `Number.MAX_SAFE_INTEGER` the encoding stops being lossless — and
 * this **throws rather than rounding**, because a silently-rounded id or count is the worst of the
 * three outcomes and the only one nobody would notice.
 */
export function jsonBody(value: unknown): string {
	return JSON.stringify(value, (_key, v) => {
		if (typeof v !== 'bigint') return v;
		if (v > BigInt(Number.MAX_SAFE_INTEGER) || v < -BigInt(Number.MAX_SAFE_INTEGER)) {
			throw new RangeError(
				`cannot encode ${v} as a JSON number without losing precision — JSON numbers are ` +
					`doubles, and this exceeds Number.MAX_SAFE_INTEGER`,
			);
		}
		return Number(v);
	});
}
