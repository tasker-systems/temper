// palette.ts
/**
 * The graph surface's palette, on contract roles. Authored doc-types are cogmap-homed and
 * workflow doc-types context-homed; home is carried by mark SHAPE (see `marks.ts`), so colour is
 * free to mean doc-type — through core's slot binding, never a hue of the web's own.
 *
 * `[Beat D — 2026-08-20]` The territory tints, the Home door tokens, the salience
 * opacity ramp, the doc-type dimming predicate and `EDGE_KINDS` were deleted with the
 * tier model — the surface has no territories, no Home tier and no ScopeBar, and the
 * only reader of `EDGE_KINDS` was the retired legend. **Nothing here is enumerated for
 * a legend any more**: every channel is decoded in place, in the reader's own words
 * (colour → the hover card names the doc-type; shape → the node's `in` row; dash and
 * color → hovering the edge renders its label). See spec §4.2.
 */
import { type Tint, tintFor } from '@tasker-systems/temper-design/vocabulary';
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

/**
 * A doc type's colour: the categorical slot core's vocabulary binds it to, or the neutral for a
 * doc type bound nowhere (`@tasker-systems/temper-design/vocabulary`). Doc types sharing a slot are
 * told apart by their word (the hover card, the rail) and, across homes, by mark shape. Paint it
 * as `var(--tp-{tint})` written in the markup, so the colour guard reads the role.
 */
export function docTypeTint(docType: string | null): Tint {
	return tintFor(docType);
}

/**
 * Structural edge, contradicts, derived_from bridge — as contract roles, so every theme restyles
 * them. `var()` strings: apply through `style`/CSS, never an SVG presentation attribute. The rule
 * roles are hairlines too faint for a 1.4px mark, so the edges take text roles: structural
 * brighter than derived (which is also dashed), contradicts on `danger`.
 */
export const EDGE_COLORS = {
	structural: 'var(--tp-text-muted)',
	contradicts: 'var(--tp-danger)',
	derived: 'var(--tp-text-subtle)',
} as const;

/**
 * Atlas canvas ground — `surface-raised`, so the drawing reads as a region lifted from the page
 * (as the old slate did); a `var()` string, applied through `style`.
 */
export const CANVAS_BG = 'var(--tp-surface-raised)';

export function isAuthored(docType: string | null): boolean {
	return docType !== null && AUTHORED_DOC_TYPES.has(docType as AtlasDocType);
}

export interface EdgeStyle {
	color: string;
	width: number;
	dash: string | null;
	markerStart: boolean;
	markerEnd: boolean;
	/**
	 * A cross at the midpoint: contradicts' channel beyond hue. Its colour sits too close to
	 * structural's in luminance for a colour-blind reader, and dash (kind) and width (weight) are
	 * taken, so it carries a shape of its own.
	 */
	cross: boolean;
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
		cross: edge.label === 'contradicts',
	};
}
