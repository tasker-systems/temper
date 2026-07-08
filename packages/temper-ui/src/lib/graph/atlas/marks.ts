// marks.ts — pure mark-encoding decisions for the Atlas force-graph.
//
// Beat D: shape encodes the *axis*, color encodes doc-type. A cogmap facet is an
// idea in the map (circle); a context-homed resource is the work it was
// derived_from — a document (rounded square). `home`, not `doc_type`, drives the
// shape, so a steward-distilled facet whose doc_type is "session" still reads as an
// idea, while its context twin reads as a document.

export type NodeMarkShape = 'circle' | 'square';

export function nodeMarkShape(home: 'context' | 'cogmap'): NodeMarkShape {
	return home === 'cogmap' ? 'circle' : 'square';
}

import type { AtlasNode } from '$lib/types/generated/graph_atlas';

/** The a11y mirror of the two-axis field: nodes split into Ideas (cogmap facets)
 *  and Sources (context-homed work). Same `home`-drives-axis rule as the marks. */
export interface AxisGroups {
	ideas: AtlasNode[];
	sources: AtlasNode[];
}

export function groupByAxis(nodes: AtlasNode[]): AxisGroups {
	const ideas: AtlasNode[] = [];
	const sources: AtlasNode[] = [];
	for (const n of nodes) {
		if (n.home === 'cogmap') ideas.push(n);
		else sources.push(n);
	}
	return { ideas, sources };
}
