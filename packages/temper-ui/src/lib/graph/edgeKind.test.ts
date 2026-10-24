import type { Answered, EdgeKind as SharedEdgeKind } from '@tasker-systems/temper-design/graph';
import { describe, expect, it } from 'vitest';
import type { EdgeKind, Polarity } from '$lib/types/generated/graph';

/**
 * The shared edge grammar names its own `EdgeKind` and polarity, because the package reads no
 * generated wire type. These hold it to the wire's in both directions: a kind added on the wire,
 * or dropped, fails `bun run check` here rather than drawing as an unknown kind.
 */
type Same<A, B> = [A] extends [B] ? ([B] extends [A] ? true : false) : false;
const kinds: Same<EdgeKind, SharedEdgeKind> = true;
const polarities: Same<Polarity, NonNullable<Answered['polarity']>> = true;

describe("the shared edge grammar's vocabulary", () => {
	it("is the wire's, in both directions (checked by the type assertions above)", () => {
		expect([kinds, polarities]).toEqual([true, true]);
	});
});
