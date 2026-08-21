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
 * The unconnected field — where nodes that this answer connects to nothing are drawn.
 *
 * `[ruled — 2026-08-20, Pete]` Measured on the flagship question after Beat 0.5: **80 of 155 nodes
 * have degree zero**. Nothing collides and nothing is hidden, so the layout witness passes — and a
 * reader looking at 80 identical scattered discs may still not find the screen legible, which is
 * what §7 reserved a decision for. The decision is: **draw all of them, in a declared place, and
 * say what the place is.**
 *
 * Three things it deliberately is not:
 *
 * - **Not a new mark.** These are `.node-chip` like every other node. The canvas's whole vocabulary
 *   is `{node, edge}` and `GraphPage.component.test.ts` fails if a third mark class appears — which
 *   is currently what covers `navigation-never-silently-changes-kind`. A separate mark for
 *   "unconnected" would also be false: unconnected is a fact about this answer, not about the
 *   resource.
 * - **Not a ranking.** {@link packField} places in the order it is given and nothing more. §2.3
 *   ruled unranked-everything is the design and that its failure mode is a measurement rather than
 *   a licence to rank; making the field legible must not smuggle one in.
 * - **Not a bound.** Every node is still drawn. When the box genuinely cannot hold them,
 *   {@link packField} returns the remainder and the caption states it, because
 *   `legibility-is-never-bought-with-silent-omission` is exactly the clause a quiet truncation
 *   would break.
 */

/** Where the field is drawn, in the canvas's own coordinates. */
export interface FieldBox {
	x: number;
	y: number;
	width: number;
	height: number;
}

export interface Placed {
	id: string;
	x: number;
	y: number;
}

export interface PackedField {
	placed: Placed[];
	/** How many did not fit at the tightest spacing. Stated by the caption, never swallowed. */
	undrawn: number;
}

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

/** Widest and tightest centre-to-centre spacing the field will use before it starts leaving marks out. */
const IDEAL_PITCH = 26;
const MIN_PITCH = 14;

/**
 * Place ids row-major in a box, tightening the spacing until they fit.
 *
 * Deterministic and order-preserving: same ids and same box give the same coordinates, and the
 * order handed in is the order drawn. Spacing shrinks from {@link IDEAL_PITCH} toward
 * {@link MIN_PITCH} rather than the box growing, because the field shares a canvas with the
 * connected core and must not push it off screen.
 */
export function packField(ids: string[], box: FieldBox): PackedField {
	if (ids.length === 0) return { placed: [], undrawn: 0 };

	const fits = (pitch: number) => {
		const cols = Math.max(1, Math.floor(box.width / pitch));
		const rows = Math.max(1, Math.floor(box.height / pitch));
		return { cols, rows, capacity: cols * rows };
	};

	// Widest spacing that holds them all; the tightest one if none does.
	let pitch = MIN_PITCH;
	for (let p = IDEAL_PITCH; p >= MIN_PITCH; p -= 2) {
		if (fits(p).capacity >= ids.length) {
			pitch = p;
			break;
		}
	}

	const { cols, capacity } = fits(pitch);
	const drawn = ids.slice(0, capacity);

	return {
		placed: drawn.map((id, i) => ({
			id,
			x: box.x + (Math.floor(i % cols) + 0.5) * pitch,
			y: box.y + (Math.floor(i / cols) + 0.5) * pitch,
		})),
		undrawn: ids.length - drawn.length,
	};
}

/**
 * What the field is, said without a machine word in it.
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
 * @see internal/superpowers/specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §5.2
 */
export function describeUnconnected(
	unconnected: number,
	total: number,
	undrawn: number,
	corpusDegrees: (number | null)[],
): string | null {
	if (unconnected === 0) return null;

	const verb = unconnected === 1 ? 'is' : 'are';
	const elsewhere = corpusElsewhere(unconnected, corpusDegrees);

	const lead = elsewhere
		? `${unconnected} of these ${total} ${verb} not connected to anything else drawn here — ` +
			`but ${unconnected === 1 ? 'it connects' : 'each connects'} to ${elsewhere} ` +
			`elsewhere in your corpus.`
		: `${unconnected} of these ${total} ${verb} not connected to anything else in this answer.`;

	return undrawn > 0 ? `${lead} ${undrawn} of them are not drawn.` : lead;
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
 * @see internal/superpowers/specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §5.3
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
 * @see internal/superpowers/specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §5.5
 * @see internal/superpowers/specs/2026-08-21-the-handoff-and-the-arm-vocabulary-design.md §2
 */
export const armsDistinguish = (nodes: Pick<GraphNode, 'arm'>[]): boolean =>
	new Set(nodes.map((n) => n.arm)).size > 1;
