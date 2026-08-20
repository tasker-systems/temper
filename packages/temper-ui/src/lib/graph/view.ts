import type { EventTrail } from '$lib/types/generated/element_trail';
import type { BoundDeclaration } from './bound';
import type { GraphModel } from './model';
import type { Readout } from './readout';

/**
 * What the graph route's load hands its page — declared here rather than beside the load so the
 * components can name it without importing a `.server.ts`.
 *
 * Everything in it is plain JSON. Nothing carries a `bigint`: the wire's `u64` fields are widened
 * to `number` at the point they are read, because ts-rs maps `u64` to `bigint` and a bigint cannot
 * cross the server/client boundary — the same type-fidelity trap that made a correct composition
 * unable to leave the process through `JSON.stringify`.
 */

export type GraphRefusal =
	/** Places were named; none of them is readable. A refusal, never a widened answer. */
	| { kind: 'no-place-resolved'; named: number }
	/** No place and no question, and nothing readable to fall back on. */
	| { kind: 'nothing-to-ask' };

export interface GraphViewData {
	owner: string;
	question: string | null;
	/** The map whose charter supplied the question, when the reader supplied none. */
	borrowedFrom: { id: string; name: string; telosResourceId: string } | null;
	refusal: GraphRefusal | null;
	model: GraphModel;
	bound: BoundDeclaration | null;
	readout: Readout | null;
	selected: string | null;
	selectedExcerpt: string | null;
	selectedTrail: EventTrail | null;
}
