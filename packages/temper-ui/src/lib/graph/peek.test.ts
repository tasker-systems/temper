import { describe, expect, it } from 'vitest';
import { buildNeighborEntries } from './peek';
import type { GraphEdge, GraphNode } from '../types/generated/graph';

function node(partial: Partial<GraphNode> & Pick<GraphNode, 'id'>): GraphNode {
	return {
		slug: partial.id,
		title: partial.id,
		doc_type: 'research',
		aggregator: false,
		edge_count: 0,
		session_count: 0,
		excerpt: null,
		...partial
	};
}

const focus = node({ id: 'focus', doc_type: 'concept', aggregator: true });
const participantA = node({ id: 'pa', doc_type: 'research', title: 'Alpha research' });
const participantB = node({ id: 'pb', doc_type: 'task', title: 'Beta task' });
const aggregatorC = node({ id: 'ac', doc_type: 'concept', aggregator: true, title: 'Gamma concept' });

const nodes = [focus, participantA, participantB, aggregatorC];

describe('buildNeighborEntries', () => {
	it('returns one entry per edge touching the focused node', () => {
		const edges: GraphEdge[] = [
			{ source: 'focus', target: 'pa', edge_type: 'relates_to' },
			{ source: 'pb', target: 'focus', edge_type: 'depends_on' },
			{ source: 'pa', target: 'pb', edge_type: 'extends' } // does not touch focus
		];
		const out = buildNeighborEntries('focus', nodes, edges);
		expect(out).toHaveLength(2);
	});

	it('sets dir to → when focus is source, ← when focus is target', () => {
		const edges: GraphEdge[] = [
			{ source: 'focus', target: 'pa', edge_type: 'relates_to' },
			{ source: 'pb', target: 'focus', edge_type: 'depends_on' }
		];
		const out = buildNeighborEntries('focus', nodes, edges);
		const pa = out.find((e) => e.other.id === 'pa')!;
		const pb = out.find((e) => e.other.id === 'pb')!;
		expect(pa.dir).toBe('→');
		expect(pb.dir).toBe('←');
	});

	it('sorts participants before aggregators', () => {
		const edges: GraphEdge[] = [
			{ source: 'focus', target: 'ac', edge_type: 'relates_to' },
			{ source: 'focus', target: 'pa', edge_type: 'relates_to' }
		];
		const out = buildNeighborEntries('focus', nodes, edges);
		expect(out[0].other.id).toBe('pa');
		expect(out[1].other.id).toBe('ac');
	});

	it('breaks ties by edge type then by neighbor title', () => {
		const edges: GraphEdge[] = [
			{ source: 'focus', target: 'pb', edge_type: 'relates_to' },
			{ source: 'focus', target: 'pa', edge_type: 'relates_to' },
			{ source: 'focus', target: 'pa', edge_type: 'depends_on' }
		];
		const out = buildNeighborEntries('focus', nodes, edges);
		// depends_on < relates_to alphabetically, so depends_on row first.
		expect(out[0]).toMatchObject({ type: 'depends_on', other: { id: 'pa' } });
		// Then two relates_to rows, sorted by title: "Alpha" before "Beta".
		expect(out[1]).toMatchObject({ type: 'relates_to', other: { id: 'pa' } });
		expect(out[2]).toMatchObject({ type: 'relates_to', other: { id: 'pb' } });
	});

	it('drops edges whose other endpoint is missing from the node set', () => {
		const edges: GraphEdge[] = [
			{ source: 'focus', target: 'ghost', edge_type: 'relates_to' },
			{ source: 'focus', target: 'pa', edge_type: 'relates_to' }
		];
		const out = buildNeighborEntries('focus', nodes, edges);
		expect(out).toHaveLength(1);
		expect(out[0].other.id).toBe('pa');
	});

	it('returns [] when the focused node has no edges', () => {
		const edges: GraphEdge[] = [{ source: 'pa', target: 'pb', edge_type: 'relates_to' }];
		expect(buildNeighborEntries('focus', nodes, edges)).toEqual([]);
	});
});
