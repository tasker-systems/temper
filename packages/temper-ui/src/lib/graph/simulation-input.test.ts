import { describe, expect, it } from 'vitest';
import { toSimulationLinks, toSimulationNodes } from './simulation-input';
import type { GraphEdge, GraphNode } from '../types/generated/graph';

const nodes: GraphNode[] = [
	{ id: 'a', slug: 'a', title: 'A', doc_type: 'concept', edge_count: 2 },
	{ id: 'b', slug: 'b', title: 'B', doc_type: 'research', edge_count: 1 }
];

const positions = new Map([
	['a', { x: 100, y: 200 }],
	['b', { x: 300, y: 400 }]
]);

const edges: GraphEdge[] = [
	{ source: 'a', target: 'b', edge_type: 'relates_to' }
];

describe('toSimulationNodes', () => {
	it('preserves every input node', () => {
		const out = toSimulationNodes(nodes, positions);
		expect(out).toHaveLength(2);
		expect(out.map((n) => n.id)).toEqual(['a', 'b']);
	});

	it('applies seeded positions to x/y', () => {
		const out = toSimulationNodes(nodes, positions);
		const a = out.find((n) => n.id === 'a')!;
		expect(a.x).toBe(100);
		expect(a.y).toBe(200);
	});

	it('preserves the underlying GraphNode fields', () => {
		const out = toSimulationNodes(nodes, positions);
		const a = out.find((n) => n.id === 'a')!;
		expect(a.slug).toBe('a');
		expect(a.doc_type).toBe('concept');
		expect(a.edge_count).toBe(2);
	});

	it('falls back to 0,0 for nodes missing from the position map', () => {
		const out = toSimulationNodes(nodes, new Map());
		expect(out[0].x).toBe(0);
		expect(out[0].y).toBe(0);
	});
});

describe('toSimulationLinks', () => {
	it('produces one link per edge with id-based source/target', () => {
		const out = toSimulationLinks(edges);
		expect(out).toHaveLength(1);
		expect(out[0].source).toBe('a');
		expect(out[0].target).toBe('b');
	});

	it('preserves edge_type for styling', () => {
		const out = toSimulationLinks(edges);
		expect(out[0].edge_type).toBe('relates_to');
	});
});
