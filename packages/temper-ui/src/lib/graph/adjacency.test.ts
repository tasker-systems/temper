import { describe, expect, it } from 'vitest';
import { buildAdjacencyIndex, neighborsOf } from './adjacency';
import type { GraphEdge } from '../types/generated/graph';

const edges: GraphEdge[] = [
	{ source: 'a', target: 'b', edge_kind: 'near', polarity: 'forward', label: 'relates_to' },
	{ source: 'a', target: 'c', edge_kind: 'near', polarity: 'forward', label: 'depends_on' },
	{ source: 'b', target: 'c', edge_kind: 'near', polarity: 'forward', label: 'extends' }
];

describe('buildAdjacencyIndex', () => {
	it('indexes edges symmetrically', () => {
		const idx = buildAdjacencyIndex(edges);
		expect(idx.get('a')).toEqual(new Set(['b', 'c']));
		expect(idx.get('b')).toEqual(new Set(['a', 'c']));
		expect(idx.get('c')).toEqual(new Set(['a', 'b']));
	});

	it('returns an empty map for no edges', () => {
		expect(buildAdjacencyIndex([]).size).toBe(0);
	});

	it('deduplicates parallel edges', () => {
		const idx = buildAdjacencyIndex([
			{ source: 'a', target: 'b', edge_kind: 'near', polarity: 'forward', label: 'relates_to' },
			{ source: 'a', target: 'b', edge_kind: 'near', polarity: 'forward', label: 'references' }
		]);
		expect(idx.get('a')).toEqual(new Set(['b']));
	});
});

describe('neighborsOf', () => {
	it('returns direct neighbors', () => {
		const idx = buildAdjacencyIndex(edges);
		expect(Array.from(neighborsOf(idx, 'a')).sort()).toEqual(['b', 'c']);
	});

	it('returns an empty set for unknown nodes', () => {
		const idx = buildAdjacencyIndex(edges);
		expect(neighborsOf(idx, 'nonexistent').size).toBe(0);
	});
});
