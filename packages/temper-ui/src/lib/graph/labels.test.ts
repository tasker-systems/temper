import { describe, expect, it } from 'vitest';
import { shouldShowLabel, truncateLabel } from './labels';
import type { GraphNode } from '../types/generated/graph';

function makeNode(partial: Partial<GraphNode>): GraphNode {
	return {
		id: '00000000-0000-0000-0000-000000000000',
		slug: 'x',
		title: 'X',
		doc_type: 'research',
		edge_count: 0,
		...partial
	};
}

describe('truncateLabel', () => {
	it('returns the original string when shorter than the limit', () => {
		expect(truncateLabel('Short', 30)).toBe('Short');
	});

	it('truncates strings longer than the limit with an ellipsis', () => {
		const long = 'A very long concept title that absolutely must get clipped';
		const result = truncateLabel(long, 20);
		expect(result.length).toBeLessThanOrEqual(20);
		expect(result.endsWith('…')).toBe(true);
	});

	it('does not truncate exactly at the limit', () => {
		const exact = 'Exactly thirty chars long here';
		expect(exact.length).toBe(30);
		expect(truncateLabel(exact, 30)).toBe(exact);
	});
});

describe('shouldShowLabel', () => {
	it('always shows concept labels regardless of zoom', () => {
		expect(shouldShowLabel(makeNode({ doc_type: 'concept' }), 0.5)).toBe(true);
		expect(shouldShowLabel(makeNode({ doc_type: 'concept' }), 2.0)).toBe(true);
	});

	it('hides non-concept labels at low zoom', () => {
		expect(shouldShowLabel(makeNode({ doc_type: 'research' }), 1.0)).toBe(false);
	});

	it('shows non-concept labels above the zoom threshold', () => {
		expect(shouldShowLabel(makeNode({ doc_type: 'research' }), 1.6)).toBe(true);
		expect(shouldShowLabel(makeNode({ doc_type: 'task' }), 2.0)).toBe(true);
	});
});
