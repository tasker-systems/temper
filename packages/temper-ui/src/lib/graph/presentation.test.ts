import { describe, expect, test } from 'vitest';
import type { ResourceView } from '$lib/types/generated/resource_view';
import { COMPOSITION_ARMS, ENTRY_ARMS, type GraphNode } from './model';
import {
	armsDistinguish,
	describeNodeLinks,
	describeUnconnected,
	nodeMeta,
	nodeRadius,
	packField,
	partitionByConnection,
	whereOf,
} from './presentation';

const NOW = new Date('2026-08-20T12:00:00Z');

const node = (o: {
	arm?: string;
	context?: string | null;
	cogmap?: string;
	stage?: string;
	updated?: string;
	degree?: number;
	corpusDegree?: number | null;
}): GraphNode => ({
	id: 'n',
	title: 'A resource',
	doc_type: 'task',
	home: o.cogmap ? 'cogmap' : 'context',
	degree: o.degree ?? 0,
	// `null` is the composition path's honest answer: `ResourceView` carries no degree, so that
	// read cannot report one. The default is therefore the ABSENT case, never a zero.
	corpusDegree: o.corpusDegree ?? null,
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

describe('the arm is said without naming an act, by the read that produced it', () => {
	/**
	 * There is no `describeArm` to test any more, and that is the finding rather than an omission
	 * here: a global switch is a claim made in one place about screens built somewhere else, and it
	 * produced a new false label per view. What survives is the RULE it carried —
	 * `no-internal-vocabulary-is-load-bearing` — applied to every arm any read declares.
	 *
	 * @see internal/superpowers/specs/2026-08-21-the-handoff-and-the-arm-vocabulary-design.md §1, §2
	 */
	const declared = [...ENTRY_ARMS, ...COMPOSITION_ARMS];

	test.each([
		'region',
		'salience',
		'wayfind',
		'survey',
		'follow-from',
	])('no arm phrase any read declares contains %s', (word) => {
		for (const arm of declared) expect(arm.label.toLowerCase()).not.toContain(word);
	});

	test('the composition still says exactly what it always said', () => {
		expect(COMPOSITION_ARMS.map((a) => a.label)).toEqual([
			'In the places you asked about',
			'From your places',
			'Followed on from your work',
		]);
	});

	test("and no read may borrow another read's key", () => {
		// The structural half. The entry read cannot render a composition's sentence because its
		// nodes carry no key the composition declares — not because anyone remembered not to.
		const entry = new Set(ENTRY_ARMS.map((a) => a.key));
		expect(COMPOSITION_ARMS.filter((a) => entry.has(a.key))).toEqual([]);
	});
});

describe('N2 — the hover card carries node metadata, not only the title', () => {
	test('where it lives, its stage, when it moved, and how it was reached', () => {
		const rows = nodeMeta(node({ arm: 'seed', stage: 'in-progress' }), COMPOSITION_ARMS[0], NOW);

		expect(rows.map((r) => r.label)).toEqual(['in', 'stage', 'updated', 'how']);
		expect(rows[3].value).toBe('in the places you asked about');
		expect(rows[0].value).toBe('@me/temper');
		expect(rows[1].value).toBe('in-progress');
		expect(rows[2].value).toBe('2h ago');
	});

	test('a row is OMITTED when the field is absent, never rendered as a dash', () => {
		// An empty value in a metadata list reads as "this resource has no stage", which is a
		// claim. Leaving the row out says only that nothing was reported.
		const rows = nodeMeta(node({}), COMPOSITION_ARMS[2], NOW);

		expect(rows.map((r) => r.label)).not.toContain('stage');
		expect(rows.map((r) => r.value)).not.toContain('—');
		expect(rows.map((r) => r.value)).not.toContain('');
	});

	test('a resource is always at least placed and accounted for', () => {
		const rows = nodeMeta(node({ context: null }), COMPOSITION_ARMS[2], NOW);

		expect(rows.map((r) => r.label)).toContain('in');
		expect(rows.map((r) => r.label)).toContain('how');
	});

	test('an arm the model did not declare says NOTHING — this card cannot translate a key', () => {
		// The unit-level shape of the D1 ruling. `nodeMeta` used to switch on the arm and always
		// had a sentence for it, whichever read the node came from. Handed no arm, it now omits
		// the row under the same rule as every other absent field: absence is not a claim.
		const rows = nodeMeta(node({ arm: 'ranked' }), undefined, NOW);

		expect(rows.map((r) => r.label)).not.toContain('how');
		expect(rows.map((r) => r.label)).not.toContain('reached');
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
		expect(describeUnconnected(0, 155, 0, [])).toBeNull();
	});

	test('it states the count against the whole answer', () => {
		expect(describeUnconnected(80, 155, 0, [])).toBe(
			'80 of these 155 are not connected to anything else in this answer.',
		);
	});

	test('one reads as one', () => {
		expect(describeUnconnected(1, 12, 0, [])).toBe(
			'1 of these 12 is not connected to anything else in this answer.',
		);
	});

	test('an undrawn remainder is added rather than left implicit', () => {
		expect(describeUnconnected(500, 600, 120, [])).toContain('120 of them are not drawn');
	});

	test('it never names a machine concept', () => {
		const s = describeUnconnected(80, 155, 3, [])!;
		for (const word of ['degree', 'node', 'orphan', 'edge', 'graph']) {
			expect(s.toLowerCase()).not.toContain(word);
		}
	});
});

describe('the band on the ENTRY read is the hub band, and the caption says so', () => {
	/**
	 * `[measured on production — 2026-08-21]` Every node in the entry read's band carries corpus
	 * degree **≥ the cut** — it must, because the cut IS the minimum drawn degree. Measured at
	 * K=130: 26 stranded, min 11, max 87, and `Maintenance` (the most-connected resource in the
	 * corpus) among them. The old sentence is true and reads as *"connected to nothing"*.
	 *
	 * @see internal/superpowers/specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §2.1
	 */
	test('with corpus evidence it says what they ARE connected to', () => {
		// The production band at K=130: 26 marks, min 11 (the cut), max 87 (`Maintenance`).
		const band = [87, 81, 59, 46, ...Array(21).fill(20), 11];
		expect(band).toHaveLength(26);

		expect(describeUnconnected(26, 130, 0, band)).toBe(
			'26 of these 130 are not connected to anything else drawn here — but each connects to 11 to 87 things elsewhere in your corpus.',
		);
	});

	test('one reads as one, and one number reads as one number', () => {
		expect(describeUnconnected(1, 130, 0, [87])).toBe(
			'1 of these 130 is not connected to anything else drawn here — but it connects to 87 things elsewhere in your corpus.',
		);
	});

	test('a band whose members all report the same figure does not say "between"', () => {
		expect(describeUnconnected(3, 40, 0, [11, 11, 11])).toBe(
			'3 of these 40 are not connected to anything else drawn here — but each connects to 11 things elsewhere in your corpus.',
		);
	});

	test('it names no machine concept either', () => {
		const s = describeUnconnected(26, 130, 0, [87, 11])!;
		for (const word of ['degree', 'node', 'orphan', 'edge', 'graph']) {
			expect(s.toLowerCase()).not.toContain(word);
		}
	});

	test('an undrawn remainder is still added rather than left implicit', () => {
		// legibility-is-never-bought-with-silent-omission does not lapse because the lead sentence
		// changed. This arm is the one that keeps a truncated field declared.
		expect(describeUnconnected(500, 600, 120, Array(500).fill(9))).toContain(
			'120 of them are not drawn',
		);
	});
});

describe('the two reads do not say the same sentence', () => {
	/**
	 * The composition path has **no corpus degree to offer** — `ResourceView` carries none — so
	 * degree zero there may genuinely mean *no connections anywhere*. Handing that screen the
	 * entry read's wording would put a claim on it that nothing measured: the same defect one
	 * surface over. The function is TOLD what it holds; it does not assume.
	 *
	 * @see internal/superpowers/specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §5.2
	 */
	test('no evidence keeps the answer-scoped sentence, byte for byte', () => {
		expect(describeUnconnected(80, 155, 0, [])).toBe(
			'80 of these 155 are not connected to anything else in this answer.',
		);
	});

	test('all-null evidence is no evidence, not zero connections', () => {
		expect(describeUnconnected(3, 40, 0, [null, null, null])).toBe(
			'3 of these 40 are not connected to anything else in this answer.',
		);
	});

	test('PARTIAL evidence claims nothing — the weaker sentence wins', () => {
		// A mixed model cannot arise from either builder today. It falls back anyway, because the
		// direction to fail in is the one that claims less than it can prove.
		expect(describeUnconnected(3, 40, 0, [87, null, 11])).toBe(
			'3 of these 40 are not connected to anything else in this answer.',
		);
	});

	test('a figure list shorter than the band is not evidence about the band', () => {
		expect(describeUnconnected(26, 130, 0, [87])).toBe(
			'26 of these 130 are not connected to anything else in this answer.',
		);
	});

	test('a reported ZERO is honest and takes the plain sentence', () => {
		// A read that reports corpus degree 0 has said something true: this really is connected to
		// nothing. There is no "elsewhere" to point at.
		expect(describeUnconnected(2, 40, 0, [0, 0])).toBe(
			'2 of these 40 are not connected to anything else in this answer.',
		);
	});
});

describe('a row in the accessibility list never asserts "0 links" about a hub', () => {
	/**
	 * `GraphA11yList` is the first thing a screen-reader user meets, and on the entry read its
	 * first row was `Maintenance — goal in @j-cole-taylor/temper, 0 links` about a resource with
	 * 87 connections. That is the FALSE half of the defect, where the caption was merely the
	 * misleading half.
	 */
	test('a drawn connection count reads as links, as it always did', () => {
		expect(describeNodeLinks(node({ degree: 3, corpusDegree: 21 }))).toBe('3 links');
		expect(describeNodeLinks(node({ degree: 1, corpusDegree: 21 }))).toBe('1 link');
	});

	test('nothing drawn but connections elsewhere states BOTH and their relationship', () => {
		expect(describeNodeLinks(node({ degree: 0, corpusDegree: 87 }))).toBe(
			'0 drawn here · 87 in your corpus',
		);
	});

	test('nothing drawn and nothing reported stays the plain claim', () => {
		expect(describeNodeLinks(node({ degree: 0, corpusDegree: null }))).toBe('0 links');
	});

	test('a reported zero is a real zero', () => {
		expect(describeNodeLinks(node({ degree: 0, corpusDegree: 0 }))).toBe('0 links');
	});
});

describe('the hover card points at what is not on screen', () => {
	test('a stranded node carries a row naming what it connects to elsewhere', () => {
		const rows = nodeMeta(node({ degree: 0, corpusDegree: 87 }), COMPOSITION_ARMS[2], NOW);
		expect(rows).toContainEqual({ label: 'connects to', value: '87 things not drawn here' });
	});

	test('one thing reads as one thing', () => {
		const rows = nodeMeta(node({ degree: 0, corpusDegree: 1 }), COMPOSITION_ARMS[2], NOW);
		expect(rows).toContainEqual({ label: 'connects to', value: '1 thing not drawn here' });
	});

	test('a connected node gets no such row — its strokes are on the screen', () => {
		const rows = nodeMeta(node({ degree: 4, corpusDegree: 87 }), COMPOSITION_ARMS[2], NOW);
		expect(rows.map((r) => r.label)).not.toContain('connects to');
	});

	test('a node whose read reported no corpus figure claims nothing', () => {
		const rows = nodeMeta(node({ degree: 0, corpusDegree: null }), COMPOSITION_ARMS[2], NOW);
		expect(rows.map((r) => r.label)).not.toContain('connects to');
	});
});

describe('a channel that encodes a constant encodes nothing', () => {
	/**
	 * `buildEntryGraph` puts every node in one arm, so the ring fired on all 130 marks of the
	 * entry canvas and gave a reader no way to tell anything from anything. This is a property of
	 * the VIEW rather than a special case for that read: any answer returning a single arm draws
	 * no ring, correctly.
	 *
	 * Deliberately not repurposed to mark the band — the arm vocabulary is chunk D's subject.
	 *
	 * @see internal/superpowers/specs/2026-08-21-hub-stranding-is-a-telling-failure-design.md §5.5
	 */
	test('one arm across every mark distinguishes nothing', () => {
		expect(armsDistinguish([node({ arm: 'seed' }), node({ arm: 'seed' })])).toBe(false);
	});

	test('two arms is a contrast worth an ink channel', () => {
		expect(armsDistinguish([node({ arm: 'seed' }), node({ arm: 'walk' })])).toBe(true);
	});

	test('an empty canvas has nothing to distinguish', () => {
		expect(armsDistinguish([])).toBe(false);
	});
});
