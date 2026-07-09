// neighbors.test.ts
import { describe, expect, it } from 'vitest';
import { atlasNeighbors } from './neighbors';
import type { AtlasNode, AtlasEdge } from '$lib/types/generated/graph_atlas';

const node = (o: Partial<AtlasNode>): AtlasNode => ({
	id: 'x',
	title: 'X',
	doc_type: null,
	home: 'context',
	degree: 0,
	salience: null,
	excerpt: null,
	stage: null,
	...o
});
const edge = (o: Partial<AtlasEdge>): AtlasEdge => ({
	id: 'e',
	source: 's',
	target: 't',
	edge_kind: 'contains',
	polarity: 'forward',
	label: null,
	weight: 1,
	...o
});

describe('atlasNeighbors', () => {
	it('yields out/in neighbors, coalescing label ?? edge_kind', () => {
		const nodes = [node({ id: 'a', title: 'A' }), node({ id: 'b', title: 'B' })];
		const edges = [edge({ id: 'e1', source: 'a', target: 'b', label: null, edge_kind: 'contains' })];
		const r = atlasNeighbors('a', nodes, edges);
		expect(r).toEqual([{ dir: '→', label: 'contains', other: nodes[1] }]);
	});
	it('drops edges whose other end is absent', () => {
		expect(atlasNeighbors('a', [node({ id: 'a' })], [edge({ source: 'a', target: 'ghost' })])).toEqual([]);
	});
	it('sorts by label then title deterministically', () => {
		const nodes = [node({ id: 'a' }), node({ id: 'b', title: 'Beta' }), node({ id: 'c', title: 'Alpha' })];
		const edges = [
			edge({ id: 'e1', source: 'a', target: 'b', label: 'rel' }),
			edge({ id: 'e2', source: 'a', target: 'c', label: 'rel' })
		];
		expect(atlasNeighbors('a', nodes, edges).map((n) => n.other.title)).toEqual(['Alpha', 'Beta']);
	});
});
