// palette.ts
/**
 * Vivid Cartographer — the single source of truth for the graph surface's palette.
 *
 * Warm semicircle = authored/knowledge doc-types (cogmap-homed); cool semicircle =
 * workflow doc-types (context-homed). Home is carried by mark SHAPE (see `marks.ts`),
 * so hue is free to mean doc-type.
 *
 * `[Beat D — 2026-08-20]` The territory tints, the Home door tokens, the salience
 * opacity ramp, the doc-type dimming predicate and `EDGE_KINDS` were deleted with the
 * tier model — the surface has no territories, no Home tier and no ScopeBar, and the
 * only reader of `EDGE_KINDS` was the retired legend. **Nothing here is enumerated for
 * a legend any more**: every channel is decoded in place, in the reader's own words
 * (hue → the hover card names the doc-type; shape → the node's `in` row; dash and
 * color → hovering the edge renders its label). See spec §4.2.
 */
import type { EdgeKind, Polarity } from '$lib/types/generated/graph';

export type AtlasDocType =
	| 'concept'
	| 'fact'
	| 'domain'
	| 'principle'
	| 'commitment'
	| 'concern'
	| 'theme'
	| 'question'
	| 'research'
	| 'task'
	| 'session'
	| 'goal'
	| 'decision'
	| 'memory';

/** Warm/authored — rendered filled. */
export const AUTHORED_DOC_TYPES: ReadonlySet<AtlasDocType> = new Set([
	'concept',
	'fact',
	'domain',
	'principle',
	'commitment',
	'concern',
	'theme',
	'question',
]);

/** Locked dark-canvas hues (light mode adds a contrast ring, not a hue fork). */
export const DOC_TYPE_HUES: Record<AtlasDocType, string> = {
	// warm · authored
	concept: '#e8942e',
	fact: '#f7c62b',
	domain: '#d3d84e',
	principle: '#f2743a',
	commitment: '#f0533f',
	concern: '#ef5090',
	theme: '#e24fc0',
	question: '#a95cf0',
	// cool · workflow
	research: '#33b0e2',
	task: '#34cf7e',
	session: '#7ed24a',
	goal: '#3a8ae8',
	decision: '#6a6ee8',
	memory: '#2ec9b0',
};

/** Neutral for unknown/absent doc-types. */
export const FALLBACK_HUE = '#9aa5b5';

/** Structural edge gray, contradicts-red, derived_from bridge. */
export const EDGE_COLORS = {
	structural: '#8b93a5',
	contradicts: '#d98a8a',
	derived: '#5f6b86',
} as const;

/** Atlas canvas slate background. */
export const CANVAS_BG = '#1b1e26';

export function docTypeHue(docType: string | null): string {
	if (docType && docType in DOC_TYPE_HUES) return DOC_TYPE_HUES[docType as AtlasDocType];
	return FALLBACK_HUE;
}

export function isAuthored(docType: string | null): boolean {
	return docType !== null && AUTHORED_DOC_TYPES.has(docType as AtlasDocType);
}

/** CSS custom-property string (`--dt-<type>:<hex>;…`) for scoping onto the canvas root. */
export function paletteStyleVars(): string {
	return (Object.entries(DOC_TYPE_HUES) as [AtlasDocType, string][])
		.map(([type, hex]) => `--dt-${type}:${hex};`)
		.join('');
}

export interface EdgeStyle {
	color: string;
	width: number;
	dash: string | null;
	markerStart: boolean;
	markerEnd: boolean;
}

const KIND_DASH: Record<EdgeKind, string | null> = {
	contains: null,
	leads_to: '7 4',
	express: '1 4',
	near: '4 4',
};

/**
 * Canonical `EdgeKind` iteration order, derived from `KIND_DASH`'s keys so it can
 * never drift from the dash mapping — the legend's "EDGE KIND" section and any
 * other UI enumerating kinds should source the list from here, not hand-roll it.
 */

/**
 * The fields the edge grammar actually reads — structural, so both edge carriers feed it.
 *
 * `[widened — 2026-08-20]` for the successor surface, whose edges are `ViaEntry` rather than
 * `AtlasEdge`. The two agree on kind, polarity and label and disagree on exactly one field, so
 * this names the union rather than forcing an adapter that would have to invent the missing one.
 * `AtlasEdge` satisfies it as it stands.
 */
export interface EdgeMark {
	edge_kind: EdgeKind;
	polarity: Polarity;
	label: string | null;
	/**
	 * The stored `kb_edges.weight`, when the read carried one.
	 *
	 * **`null`/absent is stated rather than defaulted to 1**, because a 1 is a real weight and
	 * would render as a genuine, uniformly-thin edge — indistinguishable from a corpus whose
	 * every edge happened to be weak. `ViaEntry` carries no weight at all, so the successor's
	 * edges take {@link UNWEIGHTED_WIDTH} and say nothing about strength.
	 */
	weight?: number | null;
}

/** Stroke for an edge whose source carries no weight. Deliberately not `1` — see {@link EdgeMark}. */
export const UNWEIGHTED_WIDTH = 1.4;

/** Map an edge to its SVG style per the encoding grammar (spec C2-D6). */
export function edgeStyle(edge: EdgeMark): EdgeStyle {
	const color =
		edge.label === 'derived_from'
			? EDGE_COLORS.derived
			: edge.label === 'contradicts'
				? EDGE_COLORS.contradicts
				: EDGE_COLORS.structural;
	const dash = edge.label === 'derived_from' ? '7 4' : KIND_DASH[edge.edge_kind];
	const width = edge.weight == null ? UNWEIGHTED_WIDTH : Math.max(1, Math.min(5, edge.weight));
	const symmetric = edge.edge_kind === 'near';
	return {
		color,
		width,
		dash,
		markerStart: !symmetric && edge.polarity === 'inverse',
		markerEnd: !symmetric && edge.polarity === 'forward',
	};
}
