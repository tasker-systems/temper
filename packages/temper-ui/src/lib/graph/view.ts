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
	 *
	 * `[2026-08-21]` **This member never travels on {@link GraphViewData.refusal}.** It is a verdict
	 * about the answer that came back rather than about the address, so it arrives with the read on
	 * {@link GraphViewData.tooLittleStructure} — see that field for why the two are separate.
	 */
	| { kind: 'too-little-structure'; inScope: number };

export interface GraphViewData {
	owner: string;
	question: string | null;
	/** The map whose charter supplied the question, when the reader supplied none. */
	borrowedFrom: { id: string; name: string; telosResourceId: string } | null;
	/**
	 * The refusal a reader is given **before any read runs**, so it renders as an answer rather than
	 * as a delay.
	 *
	 * Settled, and that is the contract rather than an oversight: both members that can land here —
	 * `no-place-resolved` and `nothing-to-ask` — are decided from the address and from what the
	 * reader can see, above the read. A refusal that arrived behind a loading marker would be a
	 * delay dressed as an answer. Rung 2 is the opposite kind of verdict and streams; see
	 * {@link GraphViewData.tooLittleStructure}.
	 */
	refusal: GraphRefusal | null;
	/**
	 * The answer's marks — **streamed**, so the ask box, the borrowed-charter line and the page
	 * chrome paint before the graph is read (spec §3.2, C1).
	 *
	 * Unconditionally a promise, on every branch including the refusals — where it resolves to an
	 * empty model because no read ran. An outer `null` would add a state to a field that already has
	 * three (arriving, drawn, failed) and nothing would be able to say which of them it meant.
	 */
	model: Promise<GraphModel>;
	/**
	 * The bound line's declaration — streamed, and derived from the **same read** as {@link
	 * GraphViewData.model}.
	 *
	 * The inner `null` keeps the meaning it always had: *this answer declared no bounds*, which is
	 * true of the refusal paths and is a fact about the answer rather than a read that failed.
	 */
	bound: Promise<BoundDeclaration | null>;
	/**
	 * The reasoning panel's contents — streamed, from the same read as the two above.
	 *
	 * The inner `null` means *no composition ran*, which is the honest state of the entry read and
	 * of a traversal. It is derived from the read rather than resolved outright so that the three
	 * fields cannot disagree about whether that read answered at all: the canvas, the bound line and
	 * *Why these* are three views of one read, and they arrive together because they do.
	 */
	readout: Promise<Readout | null>;
	/**
	 * Rung 2 — *there is material here, and too little structure to draw it as a graph*.
	 *
	 * **Streamed, where {@link GraphViewData.refusal} is settled, and the split is the point.** The
	 * two differ in *when they are knowable*. The addressed refusals are decided before any read, so
	 * they must render immediately. This one is the entry read's verdict about the answer it just
	 * produced — `eligible === 0` is a number that read reports — so it cannot precede the read, and
	 * it arrives in place of the canvas it replaces.
	 *
	 * `null` on every branch that ran a composition or a walk: neither read has this axis, so
	 * neither can reach this verdict, and a promise resolving to `null` says exactly that.
	 */
	tooLittleStructure: Promise<Extract<GraphRefusal, { kind: 'too-little-structure' }> | null>;
	/**
	 * The places this answer was drawn from, named.
	 *
	 * Carried so the readout can **link** each one to its measurements. A receiver that exists but
	 * cannot be reached from the surface that displaced into it is not what
	 * `displaced-structure-remains-reachable` asks for — the clause says *remains available*, and
	 * available means the reader can get there without being told a URL.
	 */
	placesAsked: NamedPlace[];
	/**
	 * The node the rail opens on, **resolved against the model** — and therefore streamed with it.
	 *
	 * `[2026-08-21]` It used to be settled, and it could be while the model was: the load resolved
	 * `?sel=` against `read.model` before returning. Once the model streams, the only way to keep
	 * this a value is to await the model — which is the exact blocking this page exists to stop. So
	 * the resolution moved into a `.then()`; **what it resolves against did not change.** A `sel`
	 * naming something this answer does not contain still opens nothing.
	 *
	 * `GraphRead`'s subtraction type is untouched by this, and that guarantee is the reason to say
	 * so: a read branch that tries to decide `selected` is still an excess property and still fails
	 * to compile, exactly as when the field held a string.
	 */
	selected: Promise<string | null>;
	/**
	 * The selected resource's first paragraph — **streamed**, so the rail frame paints without it.
	 *
	 * Three states, three representations, and keeping them apart is the whole point (spec §5.2):
	 *
	 * - the **outer `null`** means *nothing is selected* — no read was started;
	 * - a **`null` inside** the promise means *this resource genuinely has no body*;
	 * - a **rejection** means *the read failed*, and the region says so rather than claiming
	 *   there is nothing here.
	 *
	 * The third used to be spelt as the second, on both this field and `selectedTrail`, which is
	 * exactly the conflation spec §5.1 recorded.
	 *
	 * `[2026-08-21]` The outer null is now decided from the **address** — `?sel=` absent — which is
	 * the case it was always about and the only one knowable without reading. A `sel` that is named
	 * but not drawn cannot be spelled that way any more, because whether it is drawn is knowable
	 * only after the model answers; it rejects instead, and the load says why.
	 */
	selectedExcerpt: Promise<string | null> | null;
	/** The selected node's history — streamed, on the same three-way rule as `selectedExcerpt`. */
	selectedTrail: Promise<EventTrail> | null;
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
