import type { GraphEdge, GraphNode } from '../types/generated/graph';

/**
 * One row in the peek's neighbors list.
 *
 *   - `dir`   : `'→'` when the focused node is the edge source, `'←'` when it's
 *                the target. Displayed as a small glyph in the row.
 *   - `type`  : the relationship label (free text from the edge row) shown to
 *               the left of the neighbor's name.
 *   - `other` : the full `GraphNode` for the non-focused end of the edge.
 */
export interface NeighborEntry {
	dir: '→' | '←';
	type: string;
	other: GraphNode;
}

/**
 * Build the peek's neighbors list for `focusId` — every edge touching the
 * focused node, yielded as a `NeighborEntry`. Pure: same input → same output.
 *
 * Sort order is deterministic and matches the prototype (`ResourcePeek.jsx`):
 *
 *   1. Participants before aggregators (the peek's primary affordance is
 *      drilling into leaf detail; aggregators sit at the bottom as the
 *      secondary, grouping move).
 *   2. Within each group, by edge type name ascending for stable grouping.
 *   3. Within each (aggregator-flag, type) group, by the neighbor's title
 *      ascending so repeat views don't reshuffle.
 *
 * Edges whose other endpoint is not in `nodes` (shouldn't happen in practice;
 * the server guarantees edge endpoints are in the node set) are silently
 * dropped.
 */
export function buildNeighborEntries(
	focusId: string,
	nodes: GraphNode[],
	edges: GraphEdge[]
): NeighborEntry[] {
	const byId = new Map(nodes.map((n) => [n.id, n] as const));

	const entries: NeighborEntry[] = [];
	for (const e of edges) {
		if (e.source === focusId) {
			const other = byId.get(e.target);
			if (other) entries.push({ dir: '→', type: e.label, other });
		} else if (e.target === focusId) {
			const other = byId.get(e.source);
			if (other) entries.push({ dir: '←', type: e.label, other });
		}
	}

	entries.sort((a, b) => {
		if (a.other.aggregator !== b.other.aggregator) {
			return a.other.aggregator ? 1 : -1;
		}
		const typeCmp = a.type.localeCompare(b.type);
		if (typeCmp !== 0) return typeCmp;
		return a.other.title.localeCompare(b.other.title);
	});

	return entries;
}
