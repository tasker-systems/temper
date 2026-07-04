// palette.ts
/**
 * Vivid Cartographer — the single source of truth for the Atlas graph palette.
 *
 * Warm semicircle = authored/knowledge doc-types (cogmap-homed, rendered filled);
 * cool semicircle = workflow doc-types (context-homed, rendered outline). Home is
 * carried by fill-vs-outline, so hue is free to mean doc-type. See
 * docs/superpowers/specs/2026-07-03-graph-atlas-chunk-c-ui-engine-design.md (D3–D5).
 *
 * This module is the ONLY place Atlas hues are defined. The legacy `--graph-*` /
 * `--color-graph-*` CSS vars and styling.ts NODE_COLORS belong to the old graph
 * stack and are removed in Chunk D.
 */
import type { AtlasEdge, NodeHome } from '$lib/types/generated/graph_atlas';
import type { TerritoryKind } from '$lib/types/generated/graph_territory';

export type AtlasDocType =
	| 'concept' | 'fact' | 'domain' | 'principle' | 'commitment' | 'concern' | 'theme' | 'question'
	| 'research' | 'task' | 'session' | 'goal' | 'decision' | 'memory';

/** Warm/authored — rendered filled. */
export const AUTHORED_DOC_TYPES: ReadonlySet<AtlasDocType> = new Set([
	'concept', 'fact', 'domain', 'principle', 'commitment', 'concern', 'theme', 'question'
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
	memory: '#2ec9b0'
};

/** Neutral for unknown/absent doc-types. */
export const FALLBACK_HUE = '#9aa5b5';

/** Structural edge gray, contradicts-red, derived_from bridge. */
export const EDGE_COLORS = {
	structural: '#8b93a5',
	contradicts: '#d98a8a',
	derived: '#5f6b86'
} as const;

/** Dark contrast ring applied to dots in light mode so pale hues read. */
export const LIGHT_MODE_RING = '#2a2f38';

/**
 * Low-opacity cartographic hull washes for Tier-0 territories: region is
 * warm-neutral, context is cool, cogmap is warm.
 */
export const TERRITORY_TINTS: Record<TerritoryKind, string> = {
	region: '#e0b060',
	context: '#6fa8c7',
	cogmap: '#e8942e'
};

/** Atlas canvas slate background. */
export const CANVAS_BG = '#1b1e26';

/** Team-zone rectangle fill, name label, and sub-count text. */
export const TEAM_ZONE = { fill: '#6fa8c7', label: '#9fc4d6', sub: '#5f7686' } as const;

const SALIENCE_FLOOR = 0.35;

export function docTypeHue(docType: string | null): string {
	if (docType && docType in DOC_TYPE_HUES) return DOC_TYPE_HUES[docType as AtlasDocType];
	return FALLBACK_HUE;
}

export function isAuthored(docType: string | null): boolean {
	return docType !== null && AUTHORED_DOC_TYPES.has(docType as AtlasDocType);
}

/** A node's dot mark: hue by doc-type, filled vs outline by home. */
export function nodeMark(docType: string | null, home: NodeHome): { color: string; filled: boolean } {
	return { color: docTypeHue(docType), filled: home === 'cogmap' };
}

/** Salience → opacity ramp in [0.35, 1]; null/low → floor, clamps high. */
export function salienceOpacity(salience: number | null): number {
	if (salience === null || Number.isNaN(salience)) return SALIENCE_FLOOR;
	const clamped = Math.min(1, Math.max(0, salience));
	return SALIENCE_FLOOR + (1 - SALIENCE_FLOOR) * clamped;
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

const KIND_DASH: Record<AtlasEdge['edge_kind'], string | null> = {
	contains: null,
	leads_to: '7 4',
	express: '1 4',
	near: '4 4'
};

/** Map an Atlas edge to its SVG style per the encoding grammar (spec C2-D6). */
export function edgeStyle(edge: AtlasEdge): EdgeStyle {
	const color =
		edge.label === 'derived_from'
			? EDGE_COLORS.derived
			: edge.label === 'contradicts'
				? EDGE_COLORS.contradicts
				: EDGE_COLORS.structural;
	const dash = edge.label === 'derived_from' ? '7 4' : KIND_DASH[edge.edge_kind];
	const width = Math.max(1, Math.min(5, edge.weight));
	const symmetric = edge.edge_kind === 'near';
	return {
		color,
		width,
		dash,
		markerStart: !symmetric && edge.polarity === 'inverse',
		markerEnd: !symmetric && edge.polarity === 'forward'
	};
}
