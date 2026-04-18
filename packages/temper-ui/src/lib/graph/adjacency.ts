import type { GraphEdge } from '../types/generated/graph';

export type AdjacencyIndex = Map<string, Set<string>>;

/**
 * Build a symmetric adjacency index from an edge list.
 *
 * Edges are indexed in both directions so the result answers
 * "who does node X connect to?" regardless of edge direction.
 */
export function buildAdjacencyIndex(edges: GraphEdge[]): AdjacencyIndex {
	const idx: AdjacencyIndex = new Map();
	for (const edge of edges) {
		add(idx, edge.source, edge.target);
		add(idx, edge.target, edge.source);
	}
	return idx;
}

function add(idx: AdjacencyIndex, key: string, value: string): void {
	let set = idx.get(key);
	if (!set) {
		set = new Set();
		idx.set(key, set);
	}
	set.add(value);
}

/** Return the neighbors of a node, or an empty set if the node is unknown. */
export function neighborsOf(idx: AdjacencyIndex, id: string): Set<string> {
	return idx.get(id) ?? new Set();
}
