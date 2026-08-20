import { describe, expect, test } from 'vitest';
import type { ResourceView } from '$lib/types/generated/resource_view';
import type { GraphNode, NodeArm } from './model';
import { describeArm, nodeMeta, nodeRadius, whereOf } from './presentation';

const NOW = new Date('2026-08-20T12:00:00Z');

const node = (o: {
	arm?: NodeArm;
	context?: string | null;
	cogmap?: string;
	stage?: string;
	updated?: string;
}): GraphNode => ({
	id: 'n',
	title: 'A resource',
	doc_type: 'task',
	home: o.cogmap ? 'cogmap' : 'context',
	degree: 0,
	excerpt: null,
	arm: o.arm ?? 'walk',
	resource: {
		id: 'n',
		title: 'A resource',
		doc_type_name: 'task',
		context_ref: o.cogmap ? null : 'context' in o ? o.context : '@me/temper',
		cogmap_name: o.cogmap ?? null,
		managed_meta: o.stage ? { 'temper-stage': o.stage } : {},
		updated: o.updated ?? '2026-08-20T10:00:00Z',
	} as unknown as ResourceView,
});

describe('where a resource lives', () => {
	test('a context-homed row names its context ref', () => {
		expect(whereOf(node({}).resource)).toBe('@me/temper');
	});

	test('a cogmap-homed row names its map — the halves are mutually exclusive', () => {
		expect(whereOf(node({ cogmap: 'Temper — self-cognition' }).resource)).toBe(
			'Temper — self-cognition',
		);
	});

	test('a row carrying neither says so rather than rendering blank', () => {
		expect(whereOf(node({ context: null }).resource)).toBe('home not reported');
	});
});

describe('the arm is said without naming an act', () => {
	test.each<[NodeArm, string]>([
		['seed', 'In the places you asked about'],
		['survey', 'From your places'],
		['walk', 'Followed on from your work'],
	])('%s reads as %s', (arm, expected) => {
		expect(describeArm(arm)).toBe(expected);
	});

	test.each([
		'region',
		'salience',
		'wayfind',
		'survey',
		'follow-from',
	])('no arm phrase contains %s', (word) => {
		for (const arm of ['seed', 'survey', 'walk'] as NodeArm[]) {
			expect(describeArm(arm).toLowerCase()).not.toContain(word);
		}
	});
});

describe('N2 — the hover card carries node metadata, not only the title', () => {
	test('where it lives, its stage, when it moved, and how it was reached', () => {
		const rows = nodeMeta(node({ arm: 'seed', stage: 'in-progress' }), NOW);

		expect(rows.map((r) => r.label)).toEqual(['in', 'stage', 'updated', 'reached']);
		expect(rows[0].value).toBe('@me/temper');
		expect(rows[1].value).toBe('in-progress');
		expect(rows[2].value).toBe('2h ago');
	});

	test('a row is OMITTED when the field is absent, never rendered as a dash', () => {
		// An empty value in a metadata list reads as "this resource has no stage", which is a
		// claim. Leaving the row out says only that nothing was reported.
		const rows = nodeMeta(node({}), NOW);

		expect(rows.map((r) => r.label)).not.toContain('stage');
		expect(rows.map((r) => r.value)).not.toContain('—');
		expect(rows.map((r) => r.value)).not.toContain('');
	});

	test('a resource is always at least placed and accounted for', () => {
		const rows = nodeMeta(node({ context: null }), NOW);

		expect(rows.map((r) => r.label)).toContain('in');
		expect(rows.map((r) => r.label)).toContain('reached');
	});
});

describe('a mark is sized on deduped degree', () => {
	test('an isolated node still has a mark', () => {
		expect(nodeRadius(0)).toBeGreaterThan(0);
	});

	test('the measured worst case does not run away', () => {
		// Degree 25 over distinct edges, not the 98 raw `via` entries the same node carried.
		expect(nodeRadius(25)).toBeLessThanOrEqual(16);
		expect(nodeRadius(25)).toBeGreaterThan(nodeRadius(1));
	});

	test('it is monotone, so a busier node never reads as smaller', () => {
		const radii = [0, 1, 5, 9, 25, 98].map(nodeRadius);
		expect([...radii].sort((a, b) => a - b)).toEqual(radii);
	});
});
