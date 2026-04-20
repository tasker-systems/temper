import { describe, expect, it } from 'vitest';
import { toCytoscapeElements, type CytoscapeNodeElement } from './elements';
import type { GraphEdge, GraphNode } from '../types/generated/graph';

function gnode(partial: Partial<GraphNode>): GraphNode {
	return {
		id: '00000000-0000-0000-0000-000000000000',
		slug: 'x',
		title: 'X',
		doc_type: 'research',
		aggregator: false,
		edge_count: 0,
		session_count: 0,
		...partial
	};
}

describe('toCytoscapeElements', () => {
	it('produces one node element per input node, one edge element per edge', () => {
		const nodes = [gnode({ id: 'a' }), gnode({ id: 'b' })];
		const edges: GraphEdge[] = [{ source: 'a', target: 'b', edge_type: 'relates_to' }];
		const els = toCytoscapeElements(nodes, edges);
		expect(els.filter((e) => e.group === 'nodes')).toHaveLength(2);
		expect(els.filter((e) => e.group === 'edges')).toHaveLength(1);
	});

	it('sets aggregator + participant classes from the server flag', () => {
		const nodes = [
			gnode({ id: 'c', doc_type: 'concept', aggregator: true }),
			gnode({ id: 'r', doc_type: 'research', aggregator: false })
		];
		const els = toCytoscapeElements(nodes, []);
		const byId = new Map(
			els.filter((e): e is CytoscapeNodeElement => e.group === 'nodes').map((e) => [e.data.id, e])
		);
		expect(byId.get('c')!.classes).toContain('aggregator');
		expect(byId.get('c')!.classes).toContain('type-concept');
		expect(byId.get('r')!.classes).toContain('participant');
		expect(byId.get('r')!.classes).toContain('type-research');
	});

	it('precomputes label/fontSize/widthPx/heightPx/fill per node', () => {
		const els = toCytoscapeElements(
			[gnode({ id: 'a', title: 'Hello', aggregator: false })],
			[]
		);
		const nodeData = (els[0] as CytoscapeNodeElement).data;
		expect(nodeData.label).toBe('Hello');
		expect(nodeData.fontSize).toBe(13);
		expect(nodeData.heightPx).toBe(22);
		expect(nodeData.widthPx).toBeGreaterThanOrEqual(60);
		expect(nodeData.fill).toMatch(/^#[0-9a-f]{6}$/i);
	});

	it('carries session_count forward as `sessions`', () => {
		const els = toCytoscapeElements(
			[gnode({ id: 'a', session_count: 4 })],
			[]
		);
		expect((els[0] as CytoscapeNodeElement).data.sessions).toBe(4);
	});

	it('tags each edge with its etype-<type> class and picks source color from the source node', () => {
		const nodes = [
			gnode({ id: 'a', doc_type: 'concept' }),
			gnode({ id: 'b', doc_type: 'research' })
		];
		const edges: GraphEdge[] = [{ source: 'a', target: 'b', edge_type: 'relates_to' }];
		const els = toCytoscapeElements(nodes, edges);
		const edgeEl = els.find((e) => e.group === 'edges')!;
		expect(edgeEl.classes).toBe('etype-relates_to');
		expect((edgeEl.data as { sourceFill: string }).sourceFill).toMatch(/^#[0-9a-f]{6}$/i);
	});

	it('gives edges stable unique ids', () => {
		const nodes = [gnode({ id: 'a' }), gnode({ id: 'b' })];
		const edges: GraphEdge[] = [
			{ source: 'a', target: 'b', edge_type: 'relates_to' },
			{ source: 'a', target: 'b', edge_type: 'depends_on' }
		];
		const els = toCytoscapeElements(nodes, edges);
		const edgeIds = els.filter((e) => e.group === 'edges').map((e) => e.data.id);
		expect(new Set(edgeIds).size).toBe(edgeIds.length);
	});
});
