import { describe, expect, it } from 'vitest';
import { edgeStrokeDasharray, nodeColor, nodeRadius } from './styling';
import type { EdgeType, GraphNode } from '../types/generated/graph';

function makeNode(partial: Partial<GraphNode>): GraphNode {
	return {
		id: '00000000-0000-0000-0000-000000000000',
		slug: 'x',
		title: 'X',
		doc_type: 'concept',
		edge_count: 0,
		...partial
	};
}

describe('nodeRadius', () => {
	it('returns a base radius for zero-edge nodes', () => {
		expect(nodeRadius(makeNode({ doc_type: 'research', edge_count: 0 }))).toBe(6);
	});

	it('scales up with edge count', () => {
		const r1 = nodeRadius(makeNode({ edge_count: 1 }));
		const r4 = nodeRadius(makeNode({ edge_count: 4 }));
		const r16 = nodeRadius(makeNode({ edge_count: 16 }));
		expect(r4).toBeGreaterThan(r1);
		expect(r16).toBeGreaterThan(r4);
	});

	it('gives concepts a minimum radius premium', () => {
		const conceptR = nodeRadius(makeNode({ doc_type: 'concept', edge_count: 0 }));
		const researchR = nodeRadius(makeNode({ doc_type: 'research', edge_count: 0 }));
		expect(conceptR).toBeGreaterThan(researchR);
	});
});

describe('nodeColor', () => {
	it('maps each known doc type to a distinct color', () => {
		const colors = new Set([
			nodeColor('research'),
			nodeColor('task'),
			nodeColor('session'),
			nodeColor('concept')
		]);
		expect(colors.size).toBe(4);
	});

	it('returns a fallback gray for unknown doc types', () => {
		expect(nodeColor('unknown-type')).toMatch(/^#[0-9a-f]{6}$/i);
	});
});

describe('edgeStrokeDasharray', () => {
	const cases: Array<[EdgeType, string]> = [
		['depends_on', ''],
		['extends', ''],
		['parent_of', ''],
		['preceded_by', '8,4'],
		['relates_to', '4,4'],
		['derived_from', '4,4'],
		['references', '2,3']
	];

	for (const [edge, expected] of cases) {
		it(`maps ${edge} to "${expected}"`, () => {
			expect(edgeStrokeDasharray(edge)).toBe(expected);
		});
	}
});
