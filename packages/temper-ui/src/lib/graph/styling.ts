import type { EdgeType, GraphNode } from '../types/generated/graph';

/** Visual radius for a node based on its edge count and doc type. */
export function nodeRadius(node: GraphNode): number {
	const base = node.doc_type === 'concept' ? 8 : 6;
	return base + Math.sqrt(node.edge_count) * 2;
}

const NODE_COLORS: Record<string, string> = {
	research: '#7eb8da',
	task: '#f0a870',
	session: '#82c99a',
	concept: '#d48ac7'
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
