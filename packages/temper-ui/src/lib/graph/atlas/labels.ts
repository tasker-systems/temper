/** Ids whose labels are always drawn at Tier 2: the seed plus the top-K by degree. */
export function labelAnchors(
	nodes: { id: string; degree: number }[],
	seedId: string,
	k: number,
): Set<string> {
	const ranked = nodes
		.filter((n) => n.id !== seedId)
		.sort((a, b) => b.degree - a.degree)
		.slice(0, k)
		.map((n) => n.id);
	return new Set([seedId, ...ranked]);
}

/** Truncate a title to `max` chars with a trailing ellipsis. */
export function truncateLabel(title: string, max: number): string {
	return title.length <= max ? title : `${title.slice(0, max - 1)}…`;
}

/** Greedy word-wrap into ≤ maxLines lines of ≤ cap chars; final line ellipsis-truncated. */
export function wrapLabel(text: string, cap: number, maxLines = 2): string[] {
	if (text.length <= cap) return [text];
	const words = text.split(/\s+/).filter(Boolean);
	const lines: string[] = [];
	let cur = '';
	for (let i = 0; i < words.length; i++) {
		const cand = cur ? `${cur} ${words[i]}` : words[i];
		if (cand.length <= cap || !cur) {
			cur = cand;
		} else {
			lines.push(cur);
			cur = words[i];
		}
		if (lines.length === maxLines - 1) {
			const rest = [cur, ...words.slice(i + 1)].join(' ');
			lines.push(truncateLabel(rest, cap));
			return lines;
		}
	}
	if (cur) lines.push(truncateLabel(cur, cap));
	return lines;
}

/** Salience → field intensity (0..1). Exponent > 1 widens the salient/tail separation. */
export function intensityOf(salience: number | null, maxSalience: number): number {
	if (maxSalience <= 0) return 0;
	return Math.min(1, (salience ?? 0) / maxSalience) ** 1.4;
}

/** Field-effect style from intensity: brighter fill/stroke + wider glow for salient regions. */
export function fieldStyle(intensity: number, ghost: boolean) {
	if (ghost) return { fillOpacity: 0.04, strokeOpacity: 0.2, glowPx: 0 };
	return {
		fillOpacity: 0.05 + intensity * 0.3,
		strokeOpacity: 0.25 + intensity * 0.5,
		glowPx: 1 + intensity * 11,
	};
}

/**
 * Kind-agnostic Tier-0 territory weight: regions carry a normalized `salience` (used
 * verbatim), while contexts/cogmaps carry a raw `member_count` fed through a `log1p`
 * ramp. Member counts are heavy-tailed (one goal at 108, the median near 3), so the raw
 * ratio drives every ordinary goal to the opacity floor; `log1p` compresses the head so
 * small territories stay legible. 0 still maps to 0, so empty containers ghost-render.
 * A null-salience territory with members takes the log branch (the `ad324b09` change).
 */
export function territoryWeight(t: { salience: number | null; member_count: number }): number {
	return t.salience ?? Math.log1p(Math.max(0, t.member_count));
}

/** The top-K regions by salience — the ones that draw an in-panorama label. */
export function labeledRegionIds(
	regions: { id: string; salience: number | null }[],
	k: number,
): Set<string> {
	return new Set(
		[...regions]
			.sort((a, b) => (b.salience ?? 0) - (a.salience ?? 0))
			.slice(0, k)
			.map((r) => r.id),
	);
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
