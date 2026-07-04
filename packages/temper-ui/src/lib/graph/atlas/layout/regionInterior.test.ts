import { describe, expect, it } from 'vitest';
import { packRegionMembers } from './regionInterior';
import type { RegionMember } from '$lib/types/generated/graph_territory';

const member = (id: string, affinity: number | null): RegionMember => ({
	id,
	title: id,
	doc_type: 'concept',
	affinity
});

describe('packRegionMembers', () => {
	it('returns one positioned circle per member, inside the box', () => {
		const out = packRegionMembers([member('a', 0.9), member('b', 0.3), member('c', null)], {
			width: 400,
			height: 300
		});
		expect(out).toHaveLength(3);
		for (const p of out) {
			expect(p.x - p.r).toBeGreaterThanOrEqual(0);
			expect(p.x + p.r).toBeLessThanOrEqual(400);
			expect(p.r).toBeGreaterThan(0);
		}
	});
	it('sizes radius monotonically with affinity; null floors', () => {
		const out = packRegionMembers([member('hi', 0.9), member('lo', 0.05), member('none', null)], {
			width: 400,
			height: 400
		});
		const r = (id: string) => out.find((p) => p.id === id)!.r;
		expect(r('hi')).toBeGreaterThan(r('lo'));
		expect(r('none')).toBeLessThanOrEqual(r('lo'));
	});
	it('returns [] for no members', () => {
		expect(packRegionMembers([], { width: 10, height: 10 })).toEqual([]);
	});
});
