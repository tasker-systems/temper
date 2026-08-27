import type { ResourceView } from '$lib/types/generated/resource_view';
import type { GraphArm, GraphNode } from './model';
import { relativeTime } from './relativeTime';

/**
 * How a node describes itself in the panels beside the canvas.
 *
 * Pure, and separate from `model.ts`, because these are **sentences shown to a reader** rather
 * than the data behind a mark — so they are tested as strings, and the vocabulary rule that
 * governs the readout governs them too.
 */

/**
 * Where a resource lives, in the reader's terms.
 *
 * A resource is homed by exactly one anchor, so the unused half is **absent from the wire** rather
 * than null. Reading `context_ref` first and falling through to `cogmap_name` is therefore
 * exhaustive rather than a preference — but a row that carries neither is still possible from a
 * projection that did not fill them, and it says so instead of rendering an empty cell.
 */
export function whereOf(row: ResourceView): string {
	return row.context_ref ?? row.cogmap_name ?? 'home not reported';
}

/**
 * The metadata rows a hover card carries — **N2**, and the whole point of it.
 *
 * `[N2 — 2026-08-20]` Hover used to carry the title and little else. Every node here holds its
 * whole `ResourceView`, so where it lives, what stage it is at and when it last moved are already
 * in hand: this is a projection of a row the read returned, not a second read.
 *
 * A row is **omitted when the field is absent** rather than rendered as a dash. An empty value in
 * a metadata list reads as *this resource has no stage*, which is a claim; leaving the row out
 * says only that nothing was reported.
 */
export function nodeMeta(
	node: GraphNode,
	arm: GraphArm | undefined,
	now: Date = new Date(),
): { label: string; value: string }[] {
	// Read off the NODE, not off `node.resource`: the entry read's marks are an `AtlasNode`
	// projection with no row behind them, and a panel that only works when a full row is present
	// would be blank on the screen a reader meets first.
	const rows = [{ label: 'in', value: node.homeRef ?? 'home not reported' }];
	// A mark with no stroke on this screen but connections in the corpus says so HERE, beside the
	// `0 edges` chip rather than instead of it — which is what §5.3's narrowing permits and what a
	// bare second number would not be. Omitted entirely when the read reported no corpus figure:
	// absence is not a zero, and this panel must not manufacture one.
	if (node.degree === 0 && node.corpusDegree !== null && node.corpusDegree > 0) {
		const n = node.corpusDegree;
		rows.push({
			label: 'connects to',
			value: `${n} ${n === 1 ? 'thing' : 'things'} not drawn here`,
		});
	}
	if (node.stage) rows.push({ label: 'stage', value: node.stage });
	if (node.updated) {
		rows.push({ label: 'updated', value: relativeTime(node.updated, now) });
	}
	// **The read's own word for the arm, or no row at all.** This function cannot translate a key
	// — that is the whole of the D1 ruling — so a node whose arm is not in the legend it was handed
	// says nothing, under the same rule as every other row here: absence is not a claim.
	//
	// Labelled `how`, not `reached`. `reached` is now a declared property of an ARM
	// (`GraphArm.reached`), so a row headed REACHED above a mark whose arm declares `reached: false`
	// would be a fresh instance of the contradiction this change exists to remove — and REACHED over
	// *"in the places you asked about"* is the exact string a reader filed. `HOW` is what the rail
	// already calls the same fact.
	if (arm) rows.push({ label: 'how', value: arm.label.toLowerCase() });
	return rows;
}

/**
 * Mark radius from degree.
 *
 * Degree is counted over the **deduped** edge set, which is why this is safe to size on: the
 * highest-degree node in the measured walk carried 98 `via` entries over 25 distinct edges, so
 * sizing on the raw count would have inflated it fourfold and made the hub look like an outlier
 * of the reader's corpus rather than of the walk's bookkeeping.
 */
export const nodeRadius = (degree: number): number => 7 + Math.min(9, degree * 0.6);

/**
 * The unconnected band — the nodes this answer connects to nothing.
 *
 * `[ruled — 2026-08-20, Pete]` Measured on the flagship question after Beat 0.5: **80 of 155 nodes
 * have degree zero**. They were first drawn as a captioned band of marks beneath the connected
 * core, on the reasoning that every one of them should still be on the canvas.
 *
 * `[ruled — 2026-08-22, Pete]` **The marks are gone; the caption stays and opens a list.** The band
 * borrowed the canvas's mark vocabulary and dropped its semantics — on the canvas a mark's position
 * carries meaning, because a force layout puts related things near each other, and in a row it
 * carries none. A reader who has just learned to read position reads the row the same way and gets
 * a claim that is not there, which is `no-derived-thing-poses-as-authored` in the register's own
 * terms: an aggregate borrowing the visual grammar of the things it summarizes.
 *
 * The ruling applies a precedent already set one rung up, rather than arguing it again: *"dots a
 * reader cannot use are not more honest than a sentence."* So the band is now a sentence and a
 * disclosure that opens a list — title, doc type, and how many things each connects to elsewhere.
 *
 * Two things that did NOT change, and one that did:
 *
 * - **Still not a ranking.** The list is rendered in the order the answer returned. §2.3 ruled
 *   unranked-everything is the design and that its failure mode is a measurement rather than a
 *   licence to rank.
 * - **Still not a bound.** Every member is in the list. What went away with the marks is the
 *   packing that could not fit them all — so `legibility-is-never-bought-with-silent-omission` is
 *   now satisfied by construction rather than by a caption clause reporting a remainder.
 * - **The caption is load-bearing and must not be dropped.** It is what closed
 *   [degree 87 draws zero links](./01a024d3-2a16-78b1-9e7e-a0e98bd87e0e): *"each connects to 11 to
 *   87 things elsewhere in your corpus"* is the sentence that stopped the screen saying `0 links`
 *   about the most connected material the reader has. The per-row figure states the same fact per
 *   item and REINFORCES it; it never replaces it.
 */

/**
 * Split the answer's nodes by whether anything in **this answer** connects them.
 *
 * Degree is over the deduped edge set, and it is a property of the answer rather than of the
 * resource: the same resource is connected in one question's answer and isolated in another's.
 */
export function partitionByConnection<T extends { degree: number }>(
	nodes: T[],
): { connected: T[]; unconnected: T[] } {
	return {
		connected: nodes.filter((n) => n.degree > 0),
		unconnected: nodes.filter((n) => n.degree === 0),
	};
}

/**
 * What the band is, said without a machine word in it.
 *
 * `no-internal-vocabulary-is-load-bearing`: not *"80 degree-zero nodes"* — *"80 of these are not
 * connected to anything else in this answer."* The reader learns a fact about their own material
 * and about this answer, and needs to hold no concept that exists because of how the system draws.
 *
 * **Two sentences, because the two reads hold different facts — and the caller says which.**
 * `corpusDegrees` is the band's corpus figures, and it is required rather than optional so a caller
 * cannot omit the declaration and inherit a claim by default.
 *
 * - **The entry read** reports one for every member, and reports them ≥ the cut *by construction*:
 *   measured on production at K=130, all 26 band members carry corpus degree ≥ 11 and one carries
 *   87. There, *"not connected to anything else"* reads as *connected to nothing* about the most
 *   connected material the reader has, so the sentence says what they ARE connected to instead.
 * - **A composition answer** reports `null` throughout — `ResourceView` carries no degree — and its
 *   band may genuinely hold resources connected to nothing anywhere. It keeps the original
 *   sentence, byte for byte, because borrowing the other one would put a claim on that screen which
 *   its own read never measured. That is this surface's own defect, one screen over.
 *
 * `[amended — 2026-08-22]` **The `undrawn` clause is gone with the marks it counted.** It read
 * *"N of them are not drawn"*, and it existed because the band's box could run out of room. The
 * band is a list now and every member is in it, so there is no remainder to report — the clause
 * would have had no true value to take.
 *
 * @see temper-artifacts:specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §5.2
 */
export function describeUnconnected(
	unconnected: number,
	total: number,
	corpusDegrees: (number | null)[],
): string | null {
	if (unconnected === 0) return null;

	const verb = unconnected === 1 ? 'is' : 'are';
	const elsewhere = corpusElsewhere(unconnected, corpusDegrees);

	return elsewhere
		? `${unconnected} of these ${total} ${verb} not connected to anything else drawn here — ` +
				`but ${unconnected === 1 ? 'it connects' : 'each connects'} to ${elsewhere} ` +
				`elsewhere in your corpus.`
		: `${unconnected} of these ${total} ${verb} not connected to anything else in this answer.`;
}

/**
 * The band's corpus connections, phrased as a range — or `null` when the read did not report them.
 *
 * **Evidence for the whole band or none at all.** A figure list shorter than the band, or holding
 * a single `null`, describes something other than what the caption is about, and the direction to
 * fail in is the one that claims less than it can prove.
 *
 * A reported **zero** is honest rather than missing: that resource really is connected to nothing,
 * and there is no elsewhere to point at. It takes the answer-scoped sentence with the rest.
 */
function corpusElsewhere(unconnected: number, corpusDegrees: (number | null)[]): string | null {
	if (corpusDegrees.length !== unconnected) return null;
	if (!corpusDegrees.every((d): d is number => d !== null && d > 0)) return null;

	const min = Math.min(...corpusDegrees);
	const max = Math.max(...corpusDegrees);
	if (min === max) return `${min} ${min === 1 ? 'thing' : 'things'}`;
	return `${min} to ${max} things`;
}

/**
 * What one row of the accessibility list says about how connected a mark is.
 *
 * `[repaired — 2026-08-21]` This list is the first thing a screen-reader user meets, and on the
 * entry read its first row read `Maintenance — goal in @j-cole-taylor/temper, 0 links` about the
 * **most-connected resource in the corpus**. The caption above the band was misleading; this was
 * flatly false, and it is the half that had to change.
 *
 * Only the row that was false changes. A mark with strokes on screen still reads `3 links`: its
 * derived count is what the reader can verify by looking, and §5.3 stands everywhere the drawn
 * figure is not zero.
 *
 * @see temper-artifacts:specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §5.3
 */
export function describeNodeLinks(node: Pick<GraphNode, 'degree' | 'corpusDegree'>): string {
	if (node.degree > 0) return `${node.degree} ${node.degree === 1 ? 'link' : 'links'}`;
	if (node.corpusDegree === null || node.corpusDegree <= 0) return '0 links';
	return `0 drawn here · ${node.corpusDegree} in your corpus`;
}

/**
 * Whether the arm channel has a contrast to carry on THIS view.
 *
 * The ring around a mark encodes which arm brought it — the reader's own material ringed, what a
 * walk reached bare. `buildEntryGraph` puts all 130 of its nodes in one arm, so the entry canvas
 * ringed **every mark** and the channel spent ink on a constant. A reader gets no help from a
 * distinction drawn everywhere, and this is a channel one has already misread once.
 *
 * Stated as a property of the view rather than a special case for the entry read: any answer that
 * returns a single arm draws no ring, which is correct on all of them.
 *
 * Deliberately **not** repurposed to mark the band, and deliberately **unchanged** by D1. The
 * contrast is counted over the nodes actually drawn rather than over `GraphModel.arms`, and that is
 * load-bearing: a read may declare an arm and return nothing for it, and a legend count would then
 * light a channel that distinguishes nothing — which is the defect, restaged one level up.
 *
 * @see temper-artifacts:specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §5.5
 * @see temper-artifacts:specs/2026-08-21-the-handoff-and-the-arm-vocabulary-design.md §2
 */
export const armsDistinguish = (nodes: Pick<GraphNode, 'arm'>[]): boolean =>
	new Set(nodes.map((n) => n.arm)).size > 1;
