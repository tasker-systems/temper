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
		excerpt: null,
		stage: null,
		...partial
	};
}

describe('toCytoscapeElements', () => {
	it('produces one node element per input node, one edge element per edge', () => {
		const nodes = [gnode({ id: 'a' }), gnode({ id: 'b' })];
		const edges: GraphEdge[] = [{ source: 'a', target: 'b', edge_kind: 'near', polarity: 'forward', label: 'relates_to' }];
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
		const edges: GraphEdge[] = [{ source: 'a', target: 'b', edge_kind: 'near', polarity: 'forward', label: 'relates_to' }];
		const els = toCytoscapeElements(nodes, edges);
		const edgeEl = els.find((e) => e.group === 'edges')!;
		expect(edgeEl.classes).toBe('etype-relates_to');
		expect((edgeEl.data as { sourceFill: string }).sourceFill).toMatch(/^#[0-9a-f]{6}$/i);
	});

	it('gives edges stable unique ids', () => {
		const nodes = [gnode({ id: 'a' }), gnode({ id: 'b' })];
		const edges: GraphEdge[] = [
			{ source: 'a', target: 'b', edge_kind: 'near', polarity: 'forward', label: 'relates_to' },
			{ source: 'a', target: 'b', edge_kind: 'near', polarity: 'forward', label: 'depends_on' }
		];
		const els = toCytoscapeElements(nodes, edges);
		const edgeIds = els.filter((e) => e.group === 'edges').map((e) => e.data.id);
		expect(new Set(edgeIds).size).toBe(edgeIds.length);
	});

	it('builds labelWithDate when the slug has a date prefix', () => {
		const els = toCytoscapeElements(
			[gnode({ id: 'a', title: 'R11 design', slug: '2026-04-17-r11-design' })],
			[]
		);
		const data = (els[0] as CytoscapeNodeElement).data;
		expect(data.dateStrip).toBe('2026-04-17');
		expect(data.labelWithDate).toBe('R11 design\n2026-04-17');
	});

	it('leaves labelWithDate null when there is no date prefix', () => {
		const els = toCytoscapeElements(
			[gnode({ id: 'a', title: 'Circuit breakers', slug: 'circuit-breakers' })],
			[]
		);
		const data = (els[0] as CytoscapeNodeElement).data;
		expect(data.dateStrip).toBeNull();
		expect(data.labelWithDate).toBeNull();
	});

	it('stacks stage under the label for tasks with a stage value', () => {
		const els = toCytoscapeElements(
			[gnode({ id: 'a', doc_type: 'task', title: 'Auth middleware', stage: 'in-progress' })],
			[]
		);
		const data = (els[0] as CytoscapeNodeElement).data;
		expect(data.stage).toBe('in-progress');
		expect(data.labelWithDateAndStage).toBe('Auth middleware\nIN-PROGRESS');
	});

	it('stacks stage below label+date when both are present', () => {
		const els = toCytoscapeElements(
			[
				gnode({
					id: 'a',
					doc_type: 'task',
					title: 'Plan q2',
					slug: '2026-04-17-plan-q2',
					stage: 'done'
				})
			],
			[]
		);
		const data = (els[0] as CytoscapeNodeElement).data;
		expect(data.labelWithDateAndStage).toBe('Plan q2\n2026-04-17\nDONE');
	});

	it('leaves labelWithDateAndStage null when stage is absent', () => {
		const els = toCytoscapeElements(
			[gnode({ id: 'a', doc_type: 'task', title: 'No stage', stage: null })],
			[]
		);
		const data = (els[0] as CytoscapeNodeElement).data;
		expect(data.stage).toBeNull();
		expect(data.labelWithDateAndStage).toBeNull();
	});
});
