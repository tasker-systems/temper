import type { GraphNode } from '../types/generated/graph';

/**
 * Display-ready fields derived from a `GraphNode`.
 *
 * Kept as a pure transform so the render layer never sees the raw
 * slug/title — it consumes `label` / `fullTitle` / `dateStrip` directly.
 * Naming rules (from kg-handoff):
 *
 *   - ISO date prefix on the slug (`YYYY-MM-DD-...`) is extracted to
 *     `dateStrip`, the rest becomes the `label` basis.
 *   - `fullTitle` is always the untruncated `title`.
 *   - `label` is the title or de-dated slug, truncated for rendering.
 */
export interface NodeDisplay {
	label: string;
	fullTitle: string;
	dateStrip: string | null;
}

const ISO_DATE_PREFIX = /^(\d{4}-\d{2}-\d{2})-(.*)$/;

/**
 * Extract an `YYYY-MM-DD` date prefix from a slug. Returns the date and the
 * remaining slug body, or `null` if the slug does not start with a date.
 */
export function extractDatePrefix(slug: string): { date: string; rest: string } | null {
	const m = ISO_DATE_PREFIX.exec(slug);
	if (!m) return null;
	return { date: m[1], rest: m[2] };
}

/**
 * Truncate a label, appending `…` if cut. Preserves strings ≤ `max`.
 *
 * Reserves one character for the ellipsis, so `truncateLabel("abcdef", 5)`
 * returns `"abcd…"` (length 5). A `max` below 2 degenerates to `…`.
 */
export function truncateLabel(text: string, max: number): string {
	if (text.length <= max) return text;
	if (max <= 1) return '…';
	return text.slice(0, max - 1) + '…';
}

/**
 * Derive the display-ready label/fullTitle/dateStrip for a node.
 *
 * Prefers the human title; falls back to the slug body (de-dated) only if the
 * title is blank. Aggregators get a higher character budget than participants
 * because they render at 19px vs 13px and occupy more canvas.
 */
export function deriveDisplay(node: GraphNode): NodeDisplay {
	const prefix = extractDatePrefix(node.slug);
	const basis = node.title && node.title.trim().length > 0 ? node.title : prefix?.rest ?? node.slug;
	const fullTitle = basis;
	const labelMax = node.aggregator ? 48 : 32;
	return {
		label: truncateLabel(basis, labelMax),
		fullTitle,
		dateStrip: prefix?.date ?? null
	};
}
