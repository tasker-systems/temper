import type { ResourceView } from '$lib/types/generated/resource_view';
import type { GraphNode, NodeArm } from './model';
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
 * What put this node on screen, said without naming an act.
 *
 * `no-internal-vocabulary-is-load-bearing` reaches here too: the reader is told *followed on from
 * your work*, never *"reached by `follow-from`"*. The three phrases are the three the bound line
 * uses, so the same partition reads the same way in both places.
 */
export function describeArm(arm: NodeArm): string {
	switch (arm) {
		case 'seed':
			return 'In the places you asked about';
		case 'survey':
			return 'From your places';
		default:
			return 'Followed on from your work';
	}
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
	now: Date = new Date(),
): { label: string; value: string }[] {
	// Read off the NODE, not off `node.resource`: the entry read's marks are an `AtlasNode`
	// projection with no row behind them, and a panel that only works when a full row is present
	// would be blank on the screen a reader meets first.
	const rows = [{ label: 'in', value: node.homeRef ?? 'home not reported' }];
	if (node.stage) rows.push({ label: 'stage', value: node.stage });
	if (node.updated) {
		rows.push({ label: 'updated', value: relativeTime(node.updated, now) });
	}
	rows.push({ label: 'reached', value: describeArm(node.arm).toLowerCase() });
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
 */
export function describeUnconnected(
	unconnected: number,
	total: number,
	undrawn: number,
): string | null {
	if (unconnected === 0) return null;

	const verb = unconnected === 1 ? 'is' : 'are';
	const lead = `${unconnected} of these ${total} ${verb} not connected to anything else in this answer.`;
	return undrawn > 0 ? `${lead} ${undrawn} of them are not drawn.` : lead;
}
