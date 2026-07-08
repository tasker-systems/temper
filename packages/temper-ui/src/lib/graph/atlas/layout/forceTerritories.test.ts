import { describe, it, expect } from 'vitest';
import { forceTerritories } from './forceTerritories';

const T = (id: string, salience: number) =>
	({ id, kind: 'region' as const, label: null, member_count: 1, salience, anchor_id: 'c' }) as unknown as import('$lib/types/generated/graph_territory').Territory;

describe('forceTerritories', () => {
	const terr = [T('a', 1.5), T('b', 0.4), T('c', 0.9)];
	const size = { width: 800, height: 400 };
	it('is deterministic (same input → identical positions)', () => {
		expect(forceTerritories(terr, size)).toEqual(forceTerritories(terr, size));
	});
	it('sizes by salience (a > c > b)', () => {
		const r = Object.fromEntries(forceTerritories(terr, size).map((p) => [p.id, p.r]));
		expect(r.a).toBeGreaterThan(r.c);
		expect(r.c).toBeGreaterThan(r.b);
	});
	it('returns one positioned entry per territory', () => {
		expect(forceTerritories(terr, size)).toHaveLength(3);
	});
});
