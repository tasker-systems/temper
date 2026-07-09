import { describe, it, expect } from 'vitest';
import {
	labelAnchors,
	truncateLabel,
	wrapLabel,
	intensityOf,
	fieldStyle,
	labeledRegionIds,
	territoryWeight
} from './labels';

describe('labelAnchors', () => {
	const nodes = [
		{ id: 'seed', degree: 1 },
		{ id: 'a', degree: 9 },
		{ id: 'b', degree: 7 },
		{ id: 'c', degree: 3 },
		{ id: 'd', degree: 2 }
	];
	it('always includes the seed plus the top-K by degree', () => {
		const set = labelAnchors(nodes, 'seed', 2);
		expect(set.has('seed')).toBe(true);
		expect(set.has('a')).toBe(true);
		expect(set.has('b')).toBe(true);
		expect(set.has('c')).toBe(false);
	});
	it('does not double-count the seed if it is high-degree', () => {
		const set = labelAnchors([{ id: 'seed', degree: 99 }, { id: 'a', degree: 5 }, { id: 'b', degree: 4 }], 'seed', 2);
		expect(set).toEqual(new Set(['seed', 'a', 'b']));
	});
});

describe('truncateLabel', () => {
	it('leaves short titles', () => expect(truncateLabel('Short', 20)).toBe('Short'));
	it('truncates with an ellipsis', () => expect(truncateLabel('A very long node title here', 10)).toBe('A very lo…'));
});

describe('wrapLabel', () => {
	it('keeps a short label on one line', () => expect(wrapLabel('Geology', 12)).toEqual(['Geology']));
	it('wraps a long label to two lines', () => expect(wrapLabel('The gap register', 8)).toEqual(['The gap', 'register']));
	it('ellipsis-truncates the final line when it overflows', () => {
		const r = wrapLabel('Narrative gravity as a runtime-recomputed field', 10);
		expect(r.length).toBe(2);
		expect(r[1].endsWith('…')).toBe(true);
	});
	it('truncates a single over-long word to one line', () => expect(wrapLabel('N-dimensional', 8)).toEqual(['N-dimen…']));
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
		const hi = fieldStyle(1, false), lo = fieldStyle(0, false), gh = fieldStyle(1, true);
		expect(hi.fillOpacity).toBeGreaterThan(lo.fillOpacity);
		expect(hi.glowPx).toBeGreaterThan(lo.glowPx);
		expect(gh.glowPx).toBe(0);
	});
});

describe('labeledRegionIds', () => {
	it('labels the top-K by salience', () => {
		const ids = labeledRegionIds([{ id: 'a', salience: 0.1 }, { id: 'b', salience: 0.9 }, { id: 'c', salience: 0.5 }], 2);
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
