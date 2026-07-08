import { describe, expect, it } from 'vitest';
import { forceNeighborhood } from './forceNeighborhood';
import type { AtlasSubgraph } from '$lib/types/generated/graph_atlas';

// Beat D: the radial force pulls context-resources (builder axis) to an outer ring
// and cogmap facets (ideas) to the center. This is deterministic (ring init, no
// random), so we can assert the mean-radius ordering.

function node(id: string, home: 'cogmap' | 'context') {
	return { id, title: id, doc_type: 'theme', home, degree: 2, excerpt: null };
}
function edge(id: string, source: string, target: string) {
	return {
		id,
		source,
		target,
		edge_kind: 'express',
		polarity: 'forward',
		label: 'derived_from',
		weight: 1
	};
}

describe('forceNeighborhood radial-by-home', () => {
	it('settles context-resources farther from center than facets', () => {
		const subgraph: AtlasSubgraph = {
			nodes: [
				node('f1', 'cogmap'),
				node('f2', 'cogmap'),
				node('c1', 'context'),
				node('c2', 'context')
			],
			// each facet derived_from a context doc; facets linked to each other
			edges: [edge('e1', 'f1', 'f2'), edge('e2', 'f1', 'c1'), edge('e3', 'f2', 'c2')]
		} as AtlasSubgraph;

		const size = { width: 800, height: 800 };
		const { nodes } = forceNeighborhood(subgraph, ['f1', 'f2'], size);
		const cx = size.width / 2;
		const cy = size.height / 2;
		const radius = (id: string) => {
			const n = nodes.find((m) => m.id === id)!;
			return Math.hypot(n.x - cx, n.y - cy);
		};
		const facetMean = (radius('f1') + radius('f2')) / 2;
		const contextMean = (radius('c1') + radius('c2')) / 2;
		expect(contextMean).toBeGreaterThan(facetMean);
	});
});
