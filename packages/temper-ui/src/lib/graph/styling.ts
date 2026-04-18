import type { EdgeType, GraphNode } from '../types/generated/graph';

/** Visual radius for a node based on its edge count and doc type. */
export function nodeRadius(node: GraphNode): number {
	const base = node.doc_type === 'concept' ? 8 : 6;
	return base + Math.sqrt(node.edge_count) * 2;
}

/**
 * Knowledge-graph doctype palette.
 *
 * Mirrors `--graph-*` in `app.css` and `--color-graph-*` in the Tailwind
 * `@theme` block. If you change a color here, change both CSS references
 * — the hex is duplicated across the three because SVG inline styles,
 * CSS vars, and Tailwind utilities each need the raw value.
 *
 * `memory` is reserved for an upcoming doctype covering builder-and-agent
 * tooling conventions (plugin preferences, subagent guidance, consistency
 * rules). The color slot is wired up now so the renderer is ready when
 * the Rust `DocType::Memory` variant lands.
 */
const NODE_COLORS: Record<string, string> = {
	research: '#7eb8da',
	task: '#f0a870',
	session: '#82c99a',
	concept: '#d48ac7',
	goal: '#f5d277',
	decision: '#c9923c',
	memory: '#8e9fc7'
};

const FALLBACK_COLOR = '#9ca3af';

/** Color for a node based on its doc type. */
export function nodeColor(docType: string): string {
	return NODE_COLORS[docType] ?? FALLBACK_COLOR;
}

const EDGE_DASH: Record<EdgeType, string> = {
	depends_on: '',
	extends: '',
	parent_of: '',
	preceded_by: '8,4',
	relates_to: '4,4',
	derived_from: '4,4',
	references: '2,3'
};

/** SVG `stroke-dasharray` value for an edge type. Empty string = solid. */
export function edgeStrokeDasharray(edgeType: EdgeType): string {
	return EDGE_DASH[edgeType] ?? '';
}
