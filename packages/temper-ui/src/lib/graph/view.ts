import type { CogmapRegulationRow, CogmapStaleness } from '$lib/types/generated/cognitive_maps';
import type { EventTrail } from '$lib/types/generated/element_trail';
import type { AnalysedRegion } from './analysis';
import type { BoundDeclaration } from './bound';
import type { NamedPlace } from './entry';
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
	| { kind: 'nothing-to-ask' }
	/**
	 * Rung 2 (spec §6): there IS material, but too little structure to draw as a graph.
	 *
	 * A different claim from every other member of this union, and it must not render like them:
	 * the reader is not being refused, they are being told the graph is the wrong instrument for
	 * this corpus and pointed at the vault's list view, which is the right one. `inScope` is how
	 * much they actually have, so the sentence can say it rather than implying emptiness.
	 *
	 * An earlier draft had this rung fall back to drawing the recency page — 200 dots and hope. It
	 * was rejected: dots a reader cannot use are not more honest than a sentence, and this is the
	 * reading that respects `the-unstructured-reader-is-never-worse-off`.
	 */
	| { kind: 'too-little-structure'; inScope: number };

export interface GraphViewData {
	owner: string;
	question: string | null;
	/** The map whose charter supplied the question, when the reader supplied none. */
	borrowedFrom: { id: string; name: string; telosResourceId: string } | null;
	refusal: GraphRefusal | null;
	model: GraphModel;
	bound: BoundDeclaration | null;
	readout: Readout | null;
	/**
	 * The places this answer was drawn from, named.
	 *
	 * Carried so the readout can **link** each one to its measurements. A receiver that exists but
	 * cannot be reached from the surface that displaced into it is not what
	 * `displaced-structure-remains-reachable` asks for — the clause says *remains available*, and
	 * available means the reader can get there without being told a URL.
	 */
	placesAsked: NamedPlace[];
	selected: string | null;
	selectedExcerpt: string | null;
	selectedTrail: EventTrail | null;
}

export type AnalysisRefusal =
	/** Places were named; none of them is readable. A refusal, never a widened answer. */
	| { kind: 'no-place-resolved'; named: number }
	/** There is nothing the reader can read, so there is nothing to measure. */
	| { kind: 'nothing-to-analyse' };

/**
 * What the analysis door's load hands its page.
 *
 * **This is the receiver** for what the successor takes off the navigational canvas, and the shape
 * of this type is where `displaced-structure-remains-reachable` is paid: the clause asks that
 * displaced structure remain *"available somewhere that declares itself as analysis rather than as
 * the reader's material"*, and every field here is the machine's arithmetic about a place — except
 * a grouping's authored label, which is the one thing on the page a person wrote.
 *
 * `place` is null in two different situations that must not be conflated: an **index** (no place
 * named, so the page offers the choices) and a **refusal** (a place was named and cannot be read).
 * The second carries a `refusal`; the first does not.
 */
export interface AnalysisViewData {
	owner: string;
	/** The one place being measured. Null on the index and on a refusal. */
	place: NamedPlace | null;
	/**
	 * Other places the reader named that this page is not measuring.
	 *
	 * Linked rather than silently dropped: the door takes one place at a time because the same
	 * quantity spans different ranges per place, and a reader who asked for three must be told
	 * which one they are looking at and how to reach the others.
	 */
	alsoNamed: NamedPlace[];
	/** Every place the reader could measure — the index, and the no-`in` entry. */
	choices: NamedPlace[];
	refusal: AnalysisRefusal | null;
	regions: AnalysedRegion[];
	/** False when the analytics-tier read did not answer: the five scalars are UNKNOWN, not absent. */
	metricsAvailable: boolean;
	/**
	 * The map-level picture. Null for a context (which has no charter and no regulation set) and
	 * for a map whose analytics read was declined — the page tells those two apart.
	 */
	map: {
		telos: { id: string; title: string | null };
		staleness: CogmapStaleness;
		regulation: CogmapRegulationRow[];
	} | null;
}
