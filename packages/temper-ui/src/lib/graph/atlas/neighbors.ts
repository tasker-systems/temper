// neighbors.ts
import type { AtlasNode, AtlasEdge } from '$lib/types/generated/graph_atlas';

export interface AtlasNeighbor {
	dir: '→' | '←';
	label: string;
	other: AtlasNode;
}

/** Atlas-native neighbors of `focusId` from a loaded slice. Unlike the old
 *  peek.ts builder, this is typed on AtlasNode/AtlasEdge, coalesces the nullable
 *  edge label to its edge_kind, and sorts by (label, title) — no aggregator sort. */
export function atlasNeighbors(focusId: string, nodes: AtlasNode[], edges: AtlasEdge[]): AtlasNeighbor[] {
	const byId = new Map(nodes.map((n) => [n.id, n] as const));
	const out: AtlasNeighbor[] = [];
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
