import { describe, expect, it, test } from 'vitest';
import {
	DEFAULT_PLACEMENT,
	fieldStyle,
	intensityOf,
	type LabelCandidate,
	labelAnchors,
	labeledRegionIds,
	placeLabels,
	territoryWeight,
	truncateLabel,
	wrapLabel,
} from './labels';

describe('labelAnchors', () => {
	const nodes = [
		{ id: 'seed', degree: 1 },
		{ id: 'a', degree: 9 },
		{ id: 'b', degree: 7 },
		{ id: 'c', degree: 3 },
		{ id: 'd', degree: 2 },
	];
	it('always includes the seed plus the top-K by degree', () => {
		const set = labelAnchors(nodes, 'seed', 2);
		expect(set.has('seed')).toBe(true);
		expect(set.has('a')).toBe(true);
		expect(set.has('b')).toBe(true);
		expect(set.has('c')).toBe(false);
	});
	it('does not double-count the seed if it is high-degree', () => {
		const set = labelAnchors(
			[
				{ id: 'seed', degree: 99 },
				{ id: 'a', degree: 5 },
				{ id: 'b', degree: 4 },
			],
			'seed',
			2,
		);
		expect(set).toEqual(new Set(['seed', 'a', 'b']));
	});
});

describe('truncateLabel', () => {
	it('leaves short titles', () => expect(truncateLabel('Short', 20)).toBe('Short'));
	it('truncates with an ellipsis', () =>
		expect(truncateLabel('A very long node title here', 10)).toBe('A very lo…'));
});

describe('wrapLabel', () => {
	it('keeps a short label on one line', () =>
		expect(wrapLabel('Geology', 12)).toEqual(['Geology']));
	it('wraps a long label to two lines', () =>
		expect(wrapLabel('The gap register', 8)).toEqual(['The gap', 'register']));
	it('ellipsis-truncates the final line when it overflows', () => {
		const r = wrapLabel('Narrative gravity as a runtime-recomputed field', 10);
		expect(r.length).toBe(2);
		expect(r[1].endsWith('…')).toBe(true);
	});
	it('truncates a single over-long word to one line', () =>
		expect(wrapLabel('N-dimensional', 8)).toEqual(['N-dimen…']));
});

describe('intensityOf', () => {
	it('maps max salience to 1 and eases the tail down', () => {
		expect(intensityOf(1, 1)).toBeCloseTo(1);
		expect(intensityOf(0.5, 1)).toBeLessThan(0.5);
		expect(intensityOf(null, 1)).toBe(0);
	});
	it('returns 0 when maxSalience is 0', () => expect(intensityOf(0.5, 0)).toBe(0));
});

describe('fieldStyle', () => {
	it('brightens + glows with intensity, stays faint for ghosts', () => {
		const hi = fieldStyle(1, false),
			lo = fieldStyle(0, false),
			gh = fieldStyle(1, true);
		expect(hi.fillOpacity).toBeGreaterThan(lo.fillOpacity);
		expect(hi.glowPx).toBeGreaterThan(lo.glowPx);
		expect(gh.glowPx).toBe(0);
	});
});

describe('labeledRegionIds', () => {
	it('labels the top-K by salience', () => {
		const ids = labeledRegionIds(
			[
				{ id: 'a', salience: 0.1 },
				{ id: 'b', salience: 0.9 },
				{ id: 'c', salience: 0.5 },
			],
			2,
		);
		expect(ids.has('b')).toBe(true);
		expect(ids.has('c')).toBe(true);
		expect(ids.has('a')).toBe(false);
	});
});

describe('territoryWeight', () => {
	it('uses a region salience verbatim — regions skip the log ramp', () => {
		expect(territoryWeight({ salience: 0.5, member_count: 99 })).toBe(0.5);
	});

	it('log1p-compresses a raw member_count', () => {
		// member counts are heavy-tailed; the raw ratio pinned ordinary goals to the floor.
		expect(territoryWeight({ salience: null, member_count: 4 })).toBe(Math.log1p(4));
	});

	it('maps an empty container to 0 so it still ghost-renders', () => {
		expect(territoryWeight({ salience: null, member_count: 0 })).toBe(0);
	});

	it('a null-salience region with members takes the log branch (behaviour change in ad324b09)', () => {
		expect(territoryWeight({ salience: null, member_count: 7 })).toBe(Math.log1p(7));
	});
});

describe('G2 — a label never lands on another label or on another mark', () => {
	const at = (
		id: string,
		x: number,
		y: number,
		degree = 1,
		title = 'A resource title',
	): LabelCandidate => ({
		id,
		x,
		y,
		r: 8,
		title,
		degree,
	});

	test('two nodes far apart both keep their labels', () => {
		const placed = placeLabels([at('a', 100, 100), at('b', 600, 400)]);

		expect(placed.map((p) => p.id).sort()).toEqual(['a', 'b']);
	});

	test('two nodes stacked on top of each other yield exactly one label', () => {
		// Which one survives is decided by clearance, not by degree: `a` outranks `b` but its
		// caption would hang straight over `b`'s mark, so `a` yields. Covering a mark hides a
		// resource; withholding a caption does not.
		const placed = placeLabels([at('a', 100, 100, 5), at('b', 102, 103, 2)]);

		expect(placed).toHaveLength(1);
	});

	test('the higher-degree node wins the contested spot', () => {
		const placed = placeLabels([at('quiet', 100, 100, 1), at('busy', 104, 100, 9)]);

		expect(placed.map((p) => p.id)).toEqual(['busy']);
	});

	test('a label is never placed over a mark that is not its own', () => {
		// `b` sits exactly where `a`'s caption would hang.
		const placed = placeLabels([at('a', 100, 100, 5), at('b', 100, 120, 4)]);

		expect(placed.map((p) => p.id)).toEqual(['b']);
	});

	test('a node always clears its OWN mark — otherwise nothing would ever be labelled', () => {
		expect(placeLabels([at('solo', 300, 300)])).toHaveLength(1);
	});

	test('placement is deterministic, so returning to a view shows the same captions', () => {
		const nodes = Array.from({ length: 40 }, (_, i) =>
			at(`n${i}`, 40 + (i % 8) * 120, 40 + Math.floor(i / 8) * 110, i % 4),
		);

		expect(placeLabels(nodes)).toEqual(placeLabels([...nodes].reverse()));
	});

	test('equal degrees break on id, not on input order', () => {
		const placed = placeLabels([at('zeta', 100, 100, 3), at('alpha', 103, 100, 3)]);

		expect(placed.map((p) => p.id)).toEqual(['alpha']);
	});

	test('at the measured worst case the canvas is captioned, not carpeted', () => {
		// 50 nodes — `follow-from`'s published ceiling — on a 1040x620 field, with the measured
		// worst-case density (one hub at degree 25). Every node is still DRAWN; what is bounded
		// is how many carry an always-on caption.
		let seed = 7;
		const rand = () => {
			seed = (seed * 1103515245 + 12345) & 0x7fffffff;
			return seed / 0x7fffffff;
		};
		const nodes = Array.from({ length: 50 }, (_, i) =>
			at(`n${i}`, 30 + rand() * 980, 30 + rand() * 560, i === 0 ? 25 : 1 + Math.floor(rand() * 4)),
		);

		const placed = placeLabels(nodes);

		expect(placed.length).toBeGreaterThan(0);
		expect(placed.length).toBeLessThanOrEqual(DEFAULT_PLACEMENT.max);
		expect(placed[0].id).toBe('n0');
		// No two placed captions share a row-and-column neighbourhood.
		for (const p of placed) {
			for (const q of placed) {
				if (p.id === q.id) continue;
				const sameBand = Math.abs(p.y - q.y) < 9;
				const closeX = Math.abs(p.x - q.x) < 20;
				expect(sameBand && closeX).toBe(false);
			}
		}
	});

	test('every candidate is still a node — dropping a caption drops no row', () => {
		const nodes = Array.from({ length: 30 }, (_, i) => at(`n${i}`, 100 + i * 3, 100, i));
		const placed = placeLabels(nodes);

		expect(placed.length).toBeLessThan(nodes.length);
		expect(new Set(placed.map((p) => p.id)).size).toBe(placed.length);
	});
});
