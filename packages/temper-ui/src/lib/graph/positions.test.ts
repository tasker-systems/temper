import { describe, expect, it } from 'vitest';
import { seedPositions, type Viewport } from './positions';
import type { DocType } from '../types/generated/doc_type';
import type { GraphEdge, GraphNode } from '../types/generated/graph';

const viewport: Viewport = { width: 800, height: 600 };

function makeNode(id: string, doc_type: DocType): GraphNode {
	return { id, slug: id, title: id, doc_type, edge_count: 0 };
}

describe('seedPositions', () => {
	it('places every input node exactly once', () => {
		const nodes = [makeNode('c1', 'concept'), makeNode('r1', 'research')];
		const edges: GraphEdge[] = [];
		const out = seedPositions(nodes, edges, viewport);
		expect(out.size).toBe(2);
		expect(out.has('c1')).toBe(true);
		expect(out.has('r1')).toBe(true);
	});

	it('positions all points within the viewport bounds', () => {
		const nodes = Array.from({ length: 10 }, (_, i) =>
			makeNode(`n${i}`, i < 3 ? 'concept' : 'research')
		);
		const edges: GraphEdge[] = [];
		const out = seedPositions(nodes, edges, viewport);
		for (const { x, y } of out.values()) {
			expect(x).toBeGreaterThanOrEqual(0);
			expect(x).toBeLessThanOrEqual(viewport.width);
			expect(y).toBeGreaterThanOrEqual(0);
			expect(y).toBeLessThanOrEqual(viewport.height);
		}
	});

	it('produces deterministic output for the same inputs', () => {
		const nodes = [makeNode('c1', 'concept'), makeNode('r1', 'research')];
		const edges: GraphEdge[] = [
			{ source: 'c1', target: 'r1', edge_type: 'relates_to' }
		];
		const a = seedPositions(nodes, edges, viewport);
		const b = seedPositions(nodes, edges, viewport);
		for (const id of ['c1', 'r1']) {
			expect(a.get(id)!.x).toBeCloseTo(b.get(id)!.x, 6);
			expect(a.get(id)!.y).toBeCloseTo(b.get(id)!.y, 6);
		}
	});

	it('spaces concepts apart from each other', () => {
		const concepts = [
			makeNode('c1', 'concept'),
			makeNode('c2', 'concept'),
			makeNode('c3', 'concept')
		];
		const out = seedPositions(concepts, [], viewport);
		const [p1, p2, p3] = [out.get('c1')!, out.get('c2')!, out.get('c3')!];
		// Different concepts should not collapse onto the same point.
		expect(Math.hypot(p2.x - p1.x, p2.y - p1.y)).toBeGreaterThan(10);
		expect(Math.hypot(p3.x - p2.x, p3.y - p2.y)).toBeGreaterThan(10);
	});
});
