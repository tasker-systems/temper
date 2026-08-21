import { describe, expect, test } from 'vitest';
import type { ResourceView } from '$lib/types/generated/resource_view';
import type { GraphNode, NodeArm } from './model';
import {
	describeArm,
	describeUnconnected,
	nodeMeta,
	nodeRadius,
	packField,
	partitionByConnection,
	whereOf,
} from './presentation';

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
	homeRef: o.cogmap ?? ('context' in o ? (o.context ?? null) : '@me/temper'),
	updated: o.updated ?? '2026-08-20T10:00:00Z',
	stage: o.stage ?? null,
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
		expect(whereOf(node({}).resource!)).toBe('@me/temper');
	});

	test('a cogmap-homed row names its map — the halves are mutually exclusive', () => {
		expect(whereOf(node({ cogmap: 'Temper — self-cognition' }).resource!)).toBe(
			'Temper — self-cognition',
		);
	});

	test('a row carrying neither says so rather than rendering blank', () => {
		expect(whereOf(node({ context: null }).resource!)).toBe('home not reported');
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

describe('the unconnected field — degree-zero nodes are declared, not scattered', () => {
	const nodes = (n: number, degree = 0) =>
		Array.from({ length: n }, (_, i) => ({ id: `n${i}`, degree }));

	test('the partition is on whether THIS answer connects the node', () => {
		const { connected, unconnected } = partitionByConnection([
			{ id: 'a', degree: 2 },
			{ id: 'b', degree: 0 },
			{ id: 'c', degree: 1 },
		]);

		expect(connected.map((n) => n.id)).toEqual(['a', 'c']);
		expect(unconnected.map((n) => n.id)).toEqual(['b']);
	});

	test('the field preserves the order it was given — no new ranking is introduced', () => {
		// §2.3 ruled unranked-everything is the design and that its failure is a measurement, not a
		// reason to pre-emptively rank. Placing these is a legibility act; it must not become one.
		const { placed } = packField(['z', 'a', 'm'], { x: 0, y: 0, width: 400, height: 200 });

		expect(placed.map((p) => p.id)).toEqual(['z', 'a', 'm']);
	});

	test('it packs row-major and stays inside its box', () => {
		const box = { x: 20, y: 500, width: 400, height: 100 };
		const { placed, undrawn } = packField(
			nodes(24).map((n) => n.id),
			box,
		);

		expect(undrawn).toBe(0);
		for (const p of placed) {
			expect(p.x).toBeGreaterThanOrEqual(box.x);
			expect(p.x).toBeLessThanOrEqual(box.x + box.width);
			expect(p.y).toBeGreaterThanOrEqual(box.y);
			expect(p.y).toBeLessThanOrEqual(box.y + box.height);
		}
		// Row-major: the second node is to the RIGHT of the first, not below it.
		expect(placed[1].y).toBe(placed[0].y);
		expect(placed[1].x).toBeGreaterThan(placed[0].x);
	});

	test('the measured real case fits whole', () => {
		// 80 of 155 nodes at degree zero on the flagship question, post Beat 0.5.
		const { placed, undrawn } = packField(
			nodes(80).map((n) => n.id),
			{ x: 0, y: 0, width: 1040, height: 120 },
		);

		expect(placed).toHaveLength(80);
		expect(undrawn).toBe(0);
	});

	test('it is deterministic — the same input places identically twice', () => {
		const box = { x: 0, y: 0, width: 500, height: 90 };
		const ids = nodes(37).map((n) => n.id);

		expect(packField(ids, box)).toEqual(packField(ids, box));
	});

	test('a field too small to hold them all reports the remainder rather than dropping it silently', () => {
		// legibility-is-never-bought-with-silent-omission. The caption then has to say so.
		const { placed, undrawn } = packField(
			nodes(500).map((n) => n.id),
			{ x: 0, y: 0, width: 100, height: 40 },
		);

		expect(placed.length).toBeGreaterThan(0);
		expect(placed.length + undrawn).toBe(500);
		expect(undrawn).toBeGreaterThan(0);
	});
});

describe('the field says what it is, in the reader’s words', () => {
	test('nothing unconnected means no caption at all', () => {
		expect(describeUnconnected(0, 155, 0)).toBeNull();
	});

	test('it states the count against the whole answer', () => {
		expect(describeUnconnected(80, 155, 0)).toBe(
			'80 of these 155 are not connected to anything else in this answer.',
		);
	});

	test('one reads as one', () => {
		expect(describeUnconnected(1, 12, 0)).toBe(
			'1 of these 12 is not connected to anything else in this answer.',
		);
	});

	test('an undrawn remainder is added rather than left implicit', () => {
		expect(describeUnconnected(500, 600, 120)).toContain('120 of them are not drawn');
	});

	test('it never names a machine concept', () => {
		const s = describeUnconnected(80, 155, 3)!;
		for (const word of ['degree', 'node', 'orphan', 'edge', 'graph']) {
			expect(s.toLowerCase()).not.toContain(word);
		}
	});
});
