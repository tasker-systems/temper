import { describe, expect, it } from 'vitest';
import { deriveDisplay, extractDatePrefix, truncateLabel } from './derive';
import type { GraphNode } from '../types/generated/graph';

function node(partial: Partial<GraphNode>): GraphNode {
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

describe('extractDatePrefix', () => {
	it('extracts ISO date + rest from a dated slug', () => {
		expect(extractDatePrefix('2026-04-17-concept-visualization')).toEqual({
			date: '2026-04-17',
			rest: 'concept-visualization'
		});
	});

	it('returns null when the slug has no date prefix', () => {
		expect(extractDatePrefix('circuit-breakers')).toBeNull();
	});

	it('returns null on malformed dates', () => {
		expect(extractDatePrefix('26-04-17-foo')).toBeNull();
		expect(extractDatePrefix('2026-4-17-foo')).toBeNull();
	});
});

describe('truncateLabel', () => {
	it('leaves strings at or below the limit untouched', () => {
		expect(truncateLabel('hello', 10)).toBe('hello');
		expect(truncateLabel('hello', 5)).toBe('hello');
	});

	it('truncates and appends ellipsis for longer strings', () => {
		const result = truncateLabel('the quick brown fox', 10);
		expect(result).toBe('the quick…');
		expect(result.length).toBe(10);
	});

	it('degenerates to lone ellipsis for tiny limits', () => {
		expect(truncateLabel('abcdef', 1)).toBe('…');
		expect(truncateLabel('abcdef', 0)).toBe('…');
	});
});

describe('deriveDisplay', () => {
	it('prefers title over slug for label basis', () => {
		const d = deriveDisplay(node({ title: 'Idempotency Keys', slug: 'idempotency-keys' }));
		expect(d.label).toBe('Idempotency Keys');
		expect(d.fullTitle).toBe('Idempotency Keys');
		expect(d.dateStrip).toBeNull();
	});

	it('extracts date prefix from dated slugs', () => {
		const d = deriveDisplay(
			node({ title: 'R11 design', slug: '2026-04-17-r11-concept-visualization' })
		);
		expect(d.dateStrip).toBe('2026-04-17');
	});

	it('falls back to de-dated slug when title is blank', () => {
		const d = deriveDisplay(node({ title: '', slug: '2026-04-17-my-research' }));
		expect(d.label).toBe('my-research');
		expect(d.fullTitle).toBe('my-research');
		expect(d.dateStrip).toBe('2026-04-17');
	});

	it('applies a larger character budget to aggregators', () => {
		const longTitle = 'A very long concept title that will absolutely exceed the participant budget';
		const agg = deriveDisplay(node({ aggregator: true, title: longTitle }));
		const part = deriveDisplay(node({ aggregator: false, title: longTitle }));
		expect(agg.label.length).toBeGreaterThan(part.label.length);
	});
});
