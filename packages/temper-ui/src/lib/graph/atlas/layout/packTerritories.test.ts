// packTerritories.test.ts
import { describe, expect, it } from 'vitest';
import { packTerritories } from './packTerritories';
import type { Territory } from '$lib/types/generated/graph_territory';

const terr = (id: string, member_count: number): Territory => ({
	id,
	kind: 'region',
	label: id,
	member_count,
	salience: 0.5,
	anchor_id: `anchor-${id}`
});

describe('packTerritories', () => {
	it('returns one positioned circle per territory, inside the box', () => {
		const out = packTerritories([terr('a', 10), terr('b', 4), terr('c', 1)], {
			width: 400,
			height: 300
		});
		expect(out).toHaveLength(3);
		for (const p of out) {
			expect(p.x - p.r).toBeGreaterThanOrEqual(0);
			expect(p.x + p.r).toBeLessThanOrEqual(400);
			expect(p.y - p.r).toBeGreaterThanOrEqual(0);
			expect(p.y + p.r).toBeLessThanOrEqual(300);
			expect(p.r).toBeGreaterThan(0);
		}
	});

	it('sizes radius monotonically with member_count', () => {
		const out = packTerritories([terr('big', 100), terr('small', 1)], {
			width: 400,
			height: 400
		});
		const big = out.find((p) => p.id === 'big')!;
		const small = out.find((p) => p.id === 'small')!;
		expect(big.r).toBeGreaterThan(small.r);
	});

	it('carries kind/label/anchor through and floors member_count at 1', () => {
		const out = packTerritories([{ ...terr('z', 0), kind: 'context', label: 'ctx' }], {
			width: 200,
			height: 200
		});
		expect(out[0]).toMatchObject({ id: 'z', kind: 'context', label: 'ctx', anchorId: 'anchor-z' });
		expect(out[0].r).toBeGreaterThan(0); // member_count 0 floored so it still gets a circle
	});

	it('returns [] for no territories', () => {
		expect(packTerritories([], { width: 10, height: 10 })).toEqual([]);
	});
});
