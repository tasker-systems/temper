import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, test } from 'vitest';
import type {
	CogmapAnalyticsRow,
	CogmapRegionMetricsRow,
	CogmapRegionRow,
} from '$lib/types/generated/cognitive_maps';
import { analyseShape, describeStaleness, distributionOf, METRICS } from './analysis';

/**
 * The analysis surface, against the shapes the deployed substrate actually sends.
 *
 * Beat A shipped a composition builder with **69 green tests and zero callers**, and the first act
 * that POSTed its output at a real `/api/query` found three defects none of those tests could see.
 * The capture underneath this file is **untrimmed** — 907 regions across two anchors — so every
 * number below is one that was measured rather than imagined.
 *
 * Several of these assertions are about the *data*, not the code, and that is deliberate. They are
 * the standing record of why this surface refuses to normalise anything: if the ranges below ever
 * become bounded and comparable, these tests fail and the refusal can be revisited on evidence.
 */
const fixture = JSON.parse(
	readFileSync(
		join(import.meta.dirname, '../../test/fixtures/graph-analysis-anchors.json'),
		'utf8',
	),
) as {
	_captured: Record<string, unknown>;
	context: { ref: string; shape: CogmapRegionRow[]; region_metrics: CogmapRegionMetricsRow[] };
	cogmap: {
		name: string;
		shape: CogmapRegionRow[];
		region_metrics: CogmapRegionMetricsRow[];
		analytics: CogmapAnalyticsRow;
	};
	cogmap_never_materialized: {
		shape: CogmapRegionRow[];
		region_metrics: CogmapRegionMetricsRow[];
		analytics: CogmapAnalyticsRow;
	};
};

const ctx = analyseShape(fixture.context.shape, fixture.context.region_metrics);
const map = analyseShape(fixture.cogmap.shape, fixture.cogmap.region_metrics);

describe('the join holds against real responses from both doors', () => {
	test('every grouping the surface tier publishes gets its analytics-tier row', () => {
		expect(ctx.regions).toHaveLength(501);
		expect(map.regions).toHaveLength(406);

		// A row that failed to join would show every scalar absent. On real data none does.
		const unjoined = (a: typeof ctx) =>
			a.regions.filter((r) =>
				METRICS.every(
					(m) => m.key === 'member_count' || m.key === 'salience' || r.values[m.key] === null,
				),
			);
		expect(unjoined(ctx)).toHaveLength(0);
		expect(unjoined(map)).toHaveLength(0);
	});

	test('both anchor kinds answer the same read, so neither needs its own branch', () => {
		// /api/contexts/{id}/region-metrics and /api/cognitive-maps/{id}/region-metrics are two
		// doors onto one anchor_region_metrics_select — the same pairing as shape.
		expect(Object.keys(ctx.regions[0].values).sort()).toEqual(
			Object.keys(map.regions[0].values).sort(),
		);
	});
});

describe('the measurements that forbid a normalised presentation', () => {
	test('two of these quantities are unbounded, so neither is a fraction', () => {
		expect(distributionOf(map.regions, 'centrality').max).toBeGreaterThan(2000);
		expect(distributionOf(map.regions, 'salience').max).toBeGreaterThan(400);
		expect(distributionOf(ctx.regions, 'salience').max).toBeGreaterThan(60);
	});

	test('the same quantity spans wildly different ranges per place — so two places never share a scale', () => {
		// This is the measured reason the route analyses ONE anchor at a time. A single ranked list
		// over both would be arithmetic on incommensurable quantities.
		const a = distributionOf(map.regions, 'centrality').max!;
		const b = distributionOf(ctx.regions, 'centrality').max!;

		expect(a / b).toBeGreaterThan(5);
	});

	test('internal tension is identically zero across every grouping of a context', () => {
		// The finding a ranked column would have destroyed: 501 rows of the same value. The surface
		// says so in one sentence instead of ordering noise.
		const d = distributionOf(ctx.regions, 'internal_tension');

		expect(d.n).toBe(501);
		expect(d.constant).toBe(true);
		expect(d.min).toBe(0);
		expect(d.max).toBe(0);
	});

	test('the two cosines occupy a narrow band at the top, where a percentage carries no signal', () => {
		for (const [a, k] of [
			[ctx, 'content_cohesion'],
			[map, 'content_cohesion'],
			[map, 'telos_alignment'],
		] as const) {
			const d = distributionOf(a.regions, k);
			expect(d.max!).toBeLessThanOrEqual(1);
			expect(d.max! - d.min!).toBeLessThan(0.45);
		}
	});

	test('values genuinely go missing, and the count is real rather than hypothetical', () => {
		expect(distributionOf(map.regions, 'content_cohesion').nulls).toBe(4);
		expect(distributionOf(ctx.regions, 'content_cohesion').nulls).toBe(13);
		expect(distributionOf(ctx.regions, 'telos_alignment').nulls).toBe(13);
	});
});

describe('the map-level readout, as it actually comes back', () => {
	test('regulation is empty on the live flagship map — an empty state, never an error', () => {
		// Measured across all four readable maps on 2026-08-20: every one returns []. If a map ever
		// grows a regulation set, this test fails and the empty-state copy can be revisited.
		expect(fixture.cogmap.analytics.regulation).toEqual([]);
	});

	test('a settled shape reads as settled', () => {
		expect(fixture.cogmap.analytics.staleness.is_stale).toBe(false);
		expect(describeStaleness(fixture.cogmap.analytics.staleness)).toContain('worked out');
	});

	test('a map that was never materialized has no regions and says exactly that', () => {
		expect(fixture.cogmap_never_materialized.shape).toHaveLength(0);
		expect(fixture.cogmap_never_materialized.analytics.staleness.materialized_at).toBeNull();
		expect(describeStaleness(fixture.cogmap_never_materialized.analytics.staleness)).toBe(
			'This shape has never been worked out.',
		);
	});

	test('a charter resource is always named, because the column is NOT NULL', () => {
		expect(fixture.cogmap.analytics.telos_resource_id).toMatch(/^[0-9a-f-]{36}$/);
	});
});

describe('the displaced payload arrives whole', () => {
	test('member count, salience and coherence are all readable for a real grouping', () => {
		// RegionHoverCard.svelte:17-19 rendered exactly these three. Beat B took them off the
		// navigational canvas; this asserts they are reachable here, on real rows rather than a
		// fixture that could not disagree.
		const withAll = map.regions.filter(
			(r) =>
				r.values.member_count !== null &&
				r.values.salience !== null &&
				r.values.content_cohesion !== null,
		);

		expect(withAll.length).toBe(402);
		expect(map.regions.every((r) => r.label !== null)).toBe(true);
	});
});
