import { describe, expect, it } from 'vitest';
import { buildCrumbEntries, COLLAPSE_THRESHOLD, type CrumbEntry } from './trail';
import type { GraphNode } from '../types/generated/graph';

function node(id: string, extra: Partial<GraphNode> = {}): GraphNode {
	return {
		id,
		slug: id,
		title: id,
		doc_type: 'research',
		aggregator: false,
		edge_count: 0,
		session_count: 0,
		excerpt: null,
		stage: null,
		...extra
	};
}

const allNodes = Array.from({ length: 8 }, (_, i) => node(`n${i}`));

function nodeIds(entries: CrumbEntry[]): Array<string | '…'> {
	return entries.map((e) => (e.kind === 'ellipsis' ? '…' : e.node.id));
}

describe('buildCrumbEntries', () => {
	it('returns [] for an empty trail', () => {
		expect(buildCrumbEntries([], allNodes)).toEqual([]);
	});

	it('yields one entry per resolved id for short trails', () => {
		const out = buildCrumbEntries(['n0', 'n1', 'n2'], allNodes);
		expect(nodeIds(out)).toEqual(['n0', 'n1', 'n2']);
	});

	it('marks the last entry as current and earlier entries as not current', () => {
		const out = buildCrumbEntries(['n0', 'n1', 'n2'], allNodes);
		const flags = out.map((e) => (e.kind === 'node' ? e.isCurrent : null));
		expect(flags).toEqual([false, false, true]);
	});

	it('preserves the original depth on every crumb', () => {
		const out = buildCrumbEntries(['n0', 'n1', 'n2'], allNodes);
		const depths = out.map((e) => (e.kind === 'node' ? e.depth : null));
		expect(depths).toEqual([0, 1, 2]);
	});

	it('drops unresolved ids silently', () => {
		const out = buildCrumbEntries(['n0', 'ghost', 'n2'], allNodes);
		expect(nodeIds(out)).toEqual(['n0', 'n2']);
		// Depths survive from the original trail positions.
		expect((out[0] as { depth: number }).depth).toBe(0);
		expect((out[1] as { depth: number }).depth).toBe(2);
	});

	it('does NOT collapse trails just below the threshold', () => {
		const trail = Array.from({ length: COLLAPSE_THRESHOLD - 1 }, (_, i) => `n${i}`);
		const out = buildCrumbEntries(trail, allNodes);
		expect(out.some((e) => e.kind === 'ellipsis')).toBe(false);
		expect(out).toHaveLength(COLLAPSE_THRESHOLD - 1);
	});

	it('collapses trails at or beyond the threshold to first › … › penult › current', () => {
		const trail = ['n0', 'n1', 'n2', 'n3', 'n4']; // length 5
		const out = buildCrumbEntries(trail, allNodes);
		expect(nodeIds(out)).toEqual(['n0', '…', 'n3', 'n4']);
		// Depths are the originals (0, 3, 4 — ellipsis has no depth).
		const depths = out
			.filter((e): e is { kind: 'node'; depth: number } & CrumbEntry => e.kind === 'node')
			.map((e) => e.depth);
		expect(depths).toEqual([0, 3, 4]);
	});

	it('collapses the same way for very long trails', () => {
		const trail = ['n0', 'n1', 'n2', 'n3', 'n4', 'n5', 'n6', 'n7'];
		const out = buildCrumbEntries(trail, allNodes);
		expect(nodeIds(out)).toEqual(['n0', '…', 'n6', 'n7']);
	});

	it('only the last crumb is marked current when collapsed', () => {
		const out = buildCrumbEntries(['n0', 'n1', 'n2', 'n3', 'n4'], allNodes);
		const currents = out.map((e) => (e.kind === 'node' ? e.isCurrent : null));
		expect(currents).toEqual([false, null, false, true]);
	});
});
