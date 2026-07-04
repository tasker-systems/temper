import { describe, expect, it } from 'vitest';
import { hullPath } from './hull';

describe('hullPath', () => {
	it('returns null for fewer than 3 points', () => {
		expect(hullPath([])).toBeNull();
		expect(hullPath([[0, 0]])).toBeNull();
		expect(hullPath([[0, 0], [1, 1]])).toBeNull();
	});
	it('returns a closed path string for a triangle', () => {
		const p = hullPath([[0, 0], [10, 0], [5, 10]]);
		expect(p).toMatch(/^M/);
		expect(p).toMatch(/Z$/);
	});
	it('padding pushes vertices outward (path differs from unpadded)', () => {
		const pts: [number, number][] = [[0, 0], [10, 0], [10, 10], [0, 10]];
		expect(hullPath(pts, 8)).not.toBe(hullPath(pts, 0));
	});
});
