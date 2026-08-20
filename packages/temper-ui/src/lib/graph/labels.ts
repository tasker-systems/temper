// labels.ts — node caption text and placement.
//
// `[Beat D — 2026-08-20]` The territory half of this module went with the tier model:
// `labelAnchors`, `wrapLabel`, `intensityOf`, `fieldStyle`, `territoryWeight` and
// `labeledRegionIds` all served regions the successor surface does not draw. What is left is
// what a node canvas needs — one truncation rule and one collision-aware placement pass.

/** Truncate a title to `max` chars with a trailing ellipsis. */
export function truncateLabel(title: string, max: number): string {
	return title.length <= max ? title : `${title.slice(0, max - 1)}…`;
}

/**
 * A node's label, as a candidate for drawing.
 *
 * `[G2 — 2026-08-20]` Collision handling was chrome polish while most of the canvas was
 * territories; the successor surface makes **every** screen nodes-and-edges at up to the walk
 * ceiling plus an unbounded funnel arm, so a label that lands on another is now the ordinary case
 * rather than the crowded one.
 */
export interface LabelCandidate {
	id: string;
	x: number;
	y: number;
	/** The mark's radius — the label hangs below it and must clear it. */
	r: number;
	title: string;
	degree: number;
}

export interface PlacedLabel {
	id: string;
	x: number;
	y: number;
	text: string;
}

export interface LabelPlacement {
	/** How many labels to try to draw at all, highest degree first. */
	max: number;
	/** Approximate advance width of one character at the render font size. */
	charWidth: number;
	lineHeight: number;
	/** Longest label, in characters, before ellipsis. */
	cap: number;
	/** Extra clearance around each reserved box. */
	padding: number;
}

export const DEFAULT_PLACEMENT: LabelPlacement = {
	max: 14,
	charWidth: 5.4,
	lineHeight: 12,
	cap: 28,
	padding: 2,
};

interface Box {
	x1: number;
	y1: number;
	x2: number;
	y2: number;
}

const overlaps = (a: Box, b: Box): boolean =>
	a.x1 < b.x2 && b.x1 < a.x2 && a.y1 < b.y2 && b.y1 < a.y2;

/**
 * Which labels can be drawn without landing on another label or on another node's mark.
 *
 * **Greedy, highest degree first, and deterministic** — ties break on `id`, because a layout the
 * reader can return to must place the same labels twice. A candidate whose box hits anything
 * already reserved is **dropped, not nudged**: moving a label away from its node is the failure
 * mode that makes a crowded graph unreadable rather than merely sparse, since a displaced label
 * reads as belonging to whatever it landed near.
 *
 * Dropping is not silent omission in the sense the register forbids: no ROW is hidden. Every node
 * is still drawn, still hoverable, still in the accessible list — only its always-on caption is
 * withheld, and the hover card is what recovers it.
 */
export function placeLabels(
	candidates: LabelCandidate[],
	opts: Partial<LabelPlacement> = {},
): PlacedLabel[] {
	const o = { ...DEFAULT_PLACEMENT, ...opts };
	// Every mark is an obstacle: a caption sitting on top of another node hides the very thing
	// this surface exists to draw. Keyed by id so a candidate is not blocked by ITS OWN disc,
	// which its label hangs directly beneath and necessarily touches.
	const marks = new Map<string, Box>(
		candidates.map((c) => [
			c.id,
			{
				x1: c.x - c.r - o.padding,
				y1: c.y - c.r - o.padding,
				x2: c.x + c.r + o.padding,
				y2: c.y + c.r + o.padding,
			},
		]),
	);

	const ranked = [...candidates].sort((a, b) => b.degree - a.degree || a.id.localeCompare(b.id));

	const taken: Box[] = [];
	const placed: PlacedLabel[] = [];
	for (const c of ranked) {
		if (placed.length >= o.max) break;
		const text = truncateLabel(c.title, o.cap);
		const halfWidth = (text.length * o.charWidth) / 2;
		const baseline = c.y + c.r + o.lineHeight;
		const box: Box = {
			x1: c.x - halfWidth - o.padding,
			y1: baseline - o.lineHeight * 0.75,
			x2: c.x + halfWidth + o.padding,
			y2: baseline + o.padding,
		};
		const hitsAMark = [...marks].some(([id, b]) => id !== c.id && overlaps(box, b));
		if (hitsAMark || taken.some((b) => overlaps(box, b))) continue;
		taken.push(box);
		placed.push({ id: c.id, x: c.x, y: baseline, text });
	}
	return placed;
}
