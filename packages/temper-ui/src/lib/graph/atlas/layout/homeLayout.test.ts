import { describe, expect, test } from 'vitest';
import { buildLensTerritories, researchLensTerritories, layoutHomeLens } from './homeLayout';
import type { AtlasHome } from '$lib/types/generated/graph_home';

const home: AtlasHome = {
	build: [
		{ id: 'c1', name: 'temper', slug: 'temper', owner_ref: '@me', resource_count: 331, last_active_at: null },
		{ id: 'c2', name: 'storyteller', slug: 'storyteller', owner_ref: '@me', resource_count: 42, last_active_at: null }
	],
	research: [
		{ id: 'm1', name: 'Self-cognition', owner_ref: 'temper', team_ids: [], region_count: 12, facet_count: 3 }
	]
};

describe('homeLayout', () => {
	test('build lens maps contexts to context-kind territories sized by resource_count', () => {
		const ts = buildLensTerritories(home);
		expect(ts.map((t) => t.kind)).toEqual(['context', 'context']);
		expect(ts[0]).toMatchObject({ id: 'c1', label: 'temper', member_count: 331, anchor_id: 'c1' });
	});

	test('research lens maps cogmaps to cogmap-kind territories sized by region_count', () => {
		const ts = researchLensTerritories(home);
		expect(ts.map((t) => t.kind)).toEqual(['cogmap']);
		expect(ts[0]).toMatchObject({ id: 'm1', label: 'Self-cognition', member_count: 12, anchor_id: 'm1' });
	});

	test('layoutHomeLens is deterministic and finite', () => {
		const ts = buildLensTerritories(home);
		const a = layoutHomeLens(ts, { width: 1280, height: 560 });
		const b = layoutHomeLens(ts, { width: 1280, height: 560 });
		expect(a).toEqual(b);
		expect(a.every((p) => Number.isFinite(p.x) && Number.isFinite(p.y))).toBe(true);
	});
});
