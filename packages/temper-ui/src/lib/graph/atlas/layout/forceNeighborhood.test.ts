import { describe, expect, it } from 'vitest';
import { forceNeighborhood } from './forceNeighborhood';
import type { AtlasEdge, AtlasNode, AtlasSubgraph } from '$lib/types/generated/graph_atlas';

const node = (id: string, degree = 1): AtlasNode => ({
	id,
	title: id,
	doc_type: 'concept',
	home: 'cogmap',
	degree,
	salience: null,
	excerpt: null
});
const edge = (source: string, target: string): AtlasEdge => ({
	id: `${source}-${target}`,
	source,
	target,
	edge_kind: 'contains',
	polarity: 'forward',
	label: null,
	weight: 1
});
const graph = (nodes: AtlasNode[], edges: AtlasEdge[]): AtlasSubgraph => ({ nodes, edges });

describe('forceNeighborhood', () => {
	it('positions every node and flags the seed(s)', () => {
		const out = forceNeighborhood(graph([node('s'), node('n1'), node('n2')], [edge('s', 'n1'), edge('s', 'n2')]), ['s'], {
			width: 600,
			height: 400
		});
		expect(out.nodes).toHaveLength(3);
		expect(out.nodes.find((n) => n.id === 's')!.isSeed).toBe(true);
		expect(out.nodes.find((n) => n.id === 'n1')!.isSeed).toBe(false);
		for (const n of out.nodes) {
			expect(Number.isFinite(n.x)).toBe(true);
			expect(Number.isFinite(n.y)).toBe(true);
		}
	});
	it('resolves edge endpoints to node objects and carries degree', () => {
		const out = forceNeighborhood(graph([node('s', 5), node('n1', 2)], [edge('s', 'n1')]), ['s'], {
			width: 600,
			height: 400
		});
		expect(out.edges).toHaveLength(1);
		expect(out.edges[0].source.id).toBe('s');
		expect(out.edges[0].target.id).toBe('n1');
		expect(out.nodes.find((n) => n.id === 's')!.degree).toBe(5);
	});
	it('drops edges whose endpoints are missing', () => {
		const out = forceNeighborhood(graph([node('s')], [edge('s', 'ghost')]), ['s'], { width: 100, height: 100 });
		expect(out.edges).toHaveLength(0);
	});
});
