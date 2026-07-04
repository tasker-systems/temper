import { describe, expect, it } from 'vitest';
import { packCogmapTerritories } from './cogmapTerritories';
import type { OrphanNode } from '$lib/types/generated/graph_territory';

const orphan = (id: string, anchor: string, degree = 1): OrphanNode => ({
	id,
	title: id,
	doc_type: 'concept',
	degree,
	anchor_id: anchor
});

describe('packCogmapTerritories', () => {
	it('groups orphans by anchor_id into one territory per cogmap', () => {
		const out = packCogmapTerritories(
			[orphan('a', 'cm1'), orphan('b', 'cm1'), orphan('c', 'cm2')],
			{ width: 800, height: 500 }
		);
		expect(out).toHaveLength(2);
		const cm1 = out.find((t) => t.cogmapId === 'cm1')!;
		expect(cm1.facetCount).toBe(2);
		expect(cm1.facets.map((f) => f.id).sort()).toEqual(['a', 'b']);
	});
	it('packs each territory inside the box and its facets inside the territory', () => {
		const out = packCogmapTerritories(
			[orphan('a', 'cm1'), orphan('b', 'cm1'), orphan('c', 'cm1')],
			{ width: 400, height: 400 }
		);
		const t = out[0];
		expect(t.x - t.r).toBeGreaterThanOrEqual(0);
		expect(t.x + t.r).toBeLessThanOrEqual(400);
		for (const f of t.facets) {
			// facet centre lies within the territory circle
			const d = Math.hypot(f.x - t.x, f.y - t.y);
			expect(d).toBeLessThanOrEqual(t.r);
		}
	});
	it('labels generically (no cogmap name in the wire) and returns [] for no orphans', () => {
		expect(packCogmapTerritories([orphan('a', 'cm1')], { width: 200, height: 200 })[0].label).toContain('cogmap');
		expect(packCogmapTerritories([], { width: 10, height: 10 })).toEqual([]);
	});
});
