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
 * color → hovering the edge renders its relations). The edge grammar itself is the shared
 * `@tasker-systems/temper-design/graph`, painted by the contract's `graph.css`. See spec §4.2.
 */
import { type Tint, tintFor } from '@tasker-systems/temper-design/vocabulary';

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
 * Atlas canvas ground — `surface-raised`, so the drawing reads as a region lifted from the page
 * (as the old slate did); a `var()` string, applied through `style`.
 */
export const CANVAS_BG = 'var(--tp-surface-raised)';

export function isAuthored(docType: string | null): boolean {
	return docType !== null && AUTHORED_DOC_TYPES.has(docType as AtlasDocType);
}
