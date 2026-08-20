// neighbors.ts
//
// `[Beat D — 2026-08-20]` The `AtlasNeighbor = Neighbor<AtlasNode>` alias existed so the
// Atlas's call sites read unchanged after this module was made generic. Those call sites
// are gone; the generic is the only form left.

export interface Neighbor<N> {
	dir: '→' | '←';
	label: string;
	other: N;
}

/** The fields a neighbour listing reads off a node — nothing about where the node came from. */
interface NeighborNode {
	id: string;
	title: string;
}

/** The fields it reads off an edge. A null label coalesces to the kind, as it always has. */
interface NeighborEdge {
	source: string;
	target: string;
	label: string | null;
	edge_kind: string;
}

/**
 * Neighbours of `focusId` in a loaded graph — coalescing the nullable edge label to its
 * `edge_kind` and sorting by (label, title), with no aggregator sort.
 *
 * `[widened — 2026-08-20]` from `AtlasNode`/`AtlasEdge` to the fields it actually reads, so the
 * successor surface's rail lists neighbours from the graph already on screen rather than issuing a
 * second read for a relationship the canvas is drawing.
 */
export function atlasNeighbors<N extends NeighborNode, E extends NeighborEdge>(
	focusId: string,
	nodes: N[],
	edges: E[],
): Neighbor<N>[] {
	const byId = new Map(nodes.map((n) => [n.id, n] as const));
	const out: Neighbor<N>[] = [];
	for (const e of edges) {
		const label = e.label ?? e.edge_kind;
		if (e.source === focusId) {
			const other = byId.get(e.target);
			if (other) out.push({ dir: '→', label, other });
		} else if (e.target === focusId) {
			const other = byId.get(e.source);
			if (other) out.push({ dir: '←', label, other });
		}
	}
	out.sort((a, b) => a.label.localeCompare(b.label) || a.other.title.localeCompare(b.other.title));
	return out;
}
