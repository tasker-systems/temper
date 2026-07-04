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

	it('sizes regions by salience and contexts by member_count', () => {
		const region = (id: string, salience: number): Territory => ({
			id,
			kind: 'region',
			label: id,
			member_count: 1,
			salience,
			anchor_id: `anchor-${id}`
		});
		const context = (id: string, member_count: number): Territory => ({
			id,
			kind: 'context',
			label: id,
			member_count,
			salience: null,
			anchor_id: `anchor-${id}`
		});
		const out = packTerritories(
			[region('hi', 0.9), region('lo', 0.1), context('big', 80), context('small', 2)],
			{ width: 500, height: 500 }
		);
		const r = (id: string) => out.find((p) => p.id === id)!;
		expect(r('hi').r).toBeGreaterThan(r('lo').r); // salience sizes regions
		expect(r('big').r).toBeGreaterThan(r('small').r); // member_count sizes contexts
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
