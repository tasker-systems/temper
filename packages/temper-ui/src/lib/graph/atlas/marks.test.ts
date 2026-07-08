import { describe, expect, it } from 'vitest';
import type { AtlasNode } from '$lib/types/generated/graph_atlas';
import { groupByAxis, nodeMarkShape } from './marks';

describe('nodeMarkShape', () => {
	it('renders cogmap facets (ideas) as circles', () => {
		expect(nodeMarkShape('cogmap')).toBe('circle');
	});

	it('renders context resources (the builder axis) as document-squares', () => {
		expect(nodeMarkShape('context')).toBe('square');
	});
});

describe('groupByAxis', () => {
	const node = (id: string, home: 'cogmap' | 'context'): AtlasNode =>
		({ id, title: id, doc_type: 'theme', home, degree: 1, excerpt: null }) as AtlasNode;

	it('splits nodes into ideas (cogmap) and sources (context)', () => {
		const { ideas, sources } = groupByAxis([
			node('f1', 'cogmap'),
			node('x1', 'context'),
			node('f2', 'cogmap')
		]);
		expect(ideas.map((n) => n.id)).toEqual(['f1', 'f2']);
		expect(sources.map((n) => n.id)).toEqual(['x1']);
	});
});
