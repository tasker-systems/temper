import { describe, expect, it, test } from 'vitest';
import { DEFAULT_PLACEMENT, type LabelCandidate, placeLabels, truncateLabel } from './labels';

// The territory half of these tests went with the functions they covered in Beat D
// (`labelAnchors`, `wrapLabel`, `intensityOf`, `fieldStyle`, `labeledRegionIds`,
// `territoryWeight`) — the successor surface draws no regions for them to describe.

describe('truncateLabel', () => {
	it('leaves short titles', () => expect(truncateLabel('Short', 20)).toBe('Short'));
	it('truncates with an ellipsis', () =>
		expect(truncateLabel('A very long node title here', 10)).toBe('A very lo…'));
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
