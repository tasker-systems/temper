// neighbors.ts
import { edgeIdentity } from './model';
//
// `[Beat D — 2026-08-20]` The `AtlasNeighbor = Neighbor<AtlasNode>` alias existed so the
// Atlas's call sites read unchanged after this module was made generic. Those call sites
// are gone; the generic is the only form left.

export interface Neighbor<N> {
	dir: '→' | '←';
	label: string;
	other: N;
	/**
	 * The EDGE's identity — {@link edgeIdentity}, the four fields the surface declares.
	 *
	 * Carried so a render can key on it. `[found on production — 2026-08-21]` the rail keyed its
	 * neighbour list on `other.id + label + dir` instead, which drops the kind: **43 of the entry
	 * read's 275 edges share a pair**, and where two of them coalesce to the same label the keys
	 * collided, Svelte threw `each_key_duplicate` and the **whole page went blank** — on 5 of 130
	 * nodes. The panel's own trail rows already carry this scar (`trail.ts`: *"keying a render on
	 * those fields collides and crashes the panel; key on this"*); the neighbour rows did not.
	 *
	 * `dir` is deliberately absent from it: the pair is already in the identity and the focus is
	 * fixed, so direction is determined rather than distinguishing.
	 */
	key: string;
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
		const key = edgeIdentity(e.source, e.target, e.edge_kind, e.label);
		if (e.source === focusId) {
			const other = byId.get(e.target);
			if (other) out.push({ dir: '→', label, other, key });
		} else if (e.target === focusId) {
			const other = byId.get(e.source);
			if (other) out.push({ dir: '←', label, other, key });
		}
	}
	out.sort((a, b) => a.label.localeCompare(b.label) || a.other.title.localeCompare(b.other.title));
	return out;
}
