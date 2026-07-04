import { describe, expect, it } from 'vitest';
import { packCogmapTerritories } from './cogmapTerritories';
import type { OrphanNode } from '$lib/types/generated/graph_territory';

const orphan = (
	id: string,
	anchor: string,
	opts: { degree?: number; anchorLabel?: string | null } = {}
): OrphanNode => ({
	id,
	title: id,
	doc_type: 'concept',
	degree: opts.degree ?? 1,
	anchor_id: anchor,
	anchor_label: opts.anchorLabel ?? null
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
	it('falls back to a generic label when anchor_label is absent, and returns [] for no orphans', () => {
		expect(packCogmapTerritories([orphan('a', 'cm1')], { width: 200, height: 200 })[0].label).toBe(
			'cogmap · 1 facets'
		);
		expect(packCogmapTerritories([], { width: 10, height: 10 })).toEqual([]);
	});
	it('uses the cogmap name in the label when anchor_label is set', () => {
		const out = packCogmapTerritories(
			[
				orphan('a', 'cm1', { anchorLabel: 'Product Strategy' }),
				orphan('b', 'cm1', { anchorLabel: 'Product Strategy' })
			],
			{ width: 200, height: 200 }
		);
		expect(out[0].label).toBe('Product Strategy · 2 facets');
	});
});
