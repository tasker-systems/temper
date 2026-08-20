import { describe, expect, test } from 'vitest';
import type { CogmapRegionMetricsRow, CogmapRegionRow } from '$lib/types/generated/cognitive_maps';
import {
	analyseShape,
	CONTEXT_HAS_NO_MAP_READOUT,
	describeConstant,
	describeGroupingCount,
	describeNulls,
	describeRange,
	describeRegulation,
	describeStaleness,
	distributionOf,
	formatValue,
	METRICS,
	reportMetrics,
} from './analysis';

const LENS = '00000000-0000-0000-0000-0000000000aa';

const shapeRow = (over: Partial<CogmapRegionRow> = {}): CogmapRegionRow => ({
	region_id: '00000000-0000-0000-0000-000000000001',
	lens_id: LENS,
	salience: 1,
	content_cohesion: 0.9,
	label: 'A grouping',
	member_count: 3,
	...over,
});

const metricRow = (over: Partial<CogmapRegionMetricsRow> = {}): CogmapRegionMetricsRow => ({
	region_id: '00000000-0000-0000-0000-000000000001',
	lens_id: LENS,
	centrality: 10,
	content_cohesion: 0.9,
	internal_tension: 0,
	reference_standing: 2,
	telos_alignment: 0.8,
	...over,
});

describe('the two reads are joined on the region AND its lens', () => {
	test('a shape row keeps its own fields and picks up the analytics-tier scalars', () => {
		const { regions, metricsAvailable } = analyseShape([shapeRow()], [metricRow()]);

		expect(metricsAvailable).toBe(true);
		expect(regions).toHaveLength(1);
		expect(regions[0].label).toBe('A grouping');
		expect(regions[0].values.member_count).toBe(3);
		expect(regions[0].values.salience).toBe(1);
		expect(regions[0].values.centrality).toBe(10);
		expect(regions[0].values.telos_alignment).toBe(0.8);
	});

	test('the lens is part of the key, not just the region', () => {
		// A region id is unique per lens today only because every anchor has exactly one lens —
		// the same "invisible only because contexts have one lens" condition the survey act names.
		// Keying on the region alone would silently pair a region with another lens's numbers.
		const other = '00000000-0000-0000-0000-0000000000bb';
		const { regions } = analyseShape(
			[shapeRow({ lens_id: LENS }), shapeRow({ lens_id: other })],
			[metricRow({ lens_id: other, centrality: 999 })],
		);

		expect(regions.find((r) => r.lensId === LENS)?.values.centrality).toBeNull();
		expect(regions.find((r) => r.lensId === other)?.values.centrality).toBe(999);
	});

	test('a shape row with no metrics row reports the scalars as absent, never as zero', () => {
		const { regions } = analyseShape([shapeRow()], []);

		expect(regions[0].values.centrality).toBeNull();
		expect(regions[0].values.internal_tension).toBeNull();
		// The reader's own counted material survives — it never came from the metrics read.
		expect(regions[0].values.member_count).toBe(3);
	});

	test('shape drives the row set — a metrics row with no shape row is not invented', () => {
		const { regions } = analyseShape(
			[shapeRow()],
			[metricRow(), metricRow({ region_id: '00000000-0000-0000-0000-0000000000ff' })],
		);

		expect(regions).toHaveLength(1);
	});

	test('a metrics read that did NOT answer is unknown, not absent', () => {
		// 501 regions each captioned "not computed" would be a claim about the substrate made on
		// evidence the surface does not have — the RegionLookup.complete posture, one read over.
		const { regions, metricsAvailable } = analyseShape([shapeRow()], null);

		expect(metricsAvailable).toBe(false);
		expect(regions[0].values.centrality).toBeNull();
	});
});

describe('a distribution situates a raw figure without normalising it', () => {
	const regions = (...cent: (number | null)[]) =>
		analyseShape(
			cent.map((_, i) => shapeRow({ region_id: `00000000-0000-0000-0000-00000000000${i}` })),
			cent.map((c, i) =>
				metricRow({ region_id: `00000000-0000-0000-0000-00000000000${i}`, centrality: c }),
			),
		).regions;

	test('nulls are excluded from the range and counted separately', () => {
		const d = distributionOf(regions(1, null, 5, 3), 'centrality');

		expect(d.n).toBe(3);
		expect(d.nulls).toBe(1);
		expect(d.min).toBe(1);
		expect(d.max).toBe(5);
		expect(d.median).toBe(3);
	});

	test('a metric with no values at all reports no range rather than a zero one', () => {
		const d = distributionOf(regions(null, null), 'centrality');

		expect(d.n).toBe(0);
		expect(d.min).toBeNull();
		expect(d.max).toBeNull();
		expect(d.constant).toBe(false);
	});

	test('a constant metric is detected, which is what a ranking would have hidden', () => {
		// Measured: internal_tension is identically 0 across all 501 regions of @me/temper.
		// An ordering over 501 zeros manufactures a rank that does not exist.
		const d = distributionOf(regions(0, 0, 0, 0), 'centrality');

		expect(d.constant).toBe(true);
	});

	test('one value is not a constant claim about a distribution', () => {
		expect(distributionOf(regions(7), 'centrality').constant).toBe(false);
	});
});

describe('nothing is presented as a score', () => {
	test('no metric renders as a percentage, a ratio or a bounded scale', () => {
		for (const v of [0.879, 1, 0, 2342.200000000001, 497.6509704834962]) {
			const out = formatValue(v);
			expect(out).not.toContain('%');
			expect(out).not.toContain('/');
			expect(out).not.toMatch(/of 1\b|out of/);
		}
	});

	test('float noise is corrected rather than printed', () => {
		expect(formatValue(2342.200000000001)).toBe('2342.2');
		expect(formatValue(303.99999999999994)).toBe('304');
		expect(formatValue(497.6509704834962)).toBe('497.651');
	});

	test('an absent value is a dash, never a zero', () => {
		expect(formatValue(null)).toBe('—');
		expect(formatValue(0)).toBe('0');
	});

	test('the range names this place and no other', () => {
		const d = { n: 3, nulls: 0, min: 0, median: 1, max: 2342.2, constant: false };

		expect(describeRange(d)).toBe('here: 0 – 2342.2');
	});

	test('a constant metric is said, not ranked', () => {
		const d = { n: 501, nulls: 0, min: 0, median: 0, max: 0, constant: true };

		expect(describeConstant(d)).toBe('Every grouping here measures 0.');
	});

	test('missing values are stated rather than left to look like zeroes', () => {
		expect(describeNulls({ n: 402, nulls: 4, min: 0, median: 1, max: 2, constant: false })).toBe(
			'4 of 406 have no value for this.',
		);
		expect(
			describeNulls({ n: 406, nulls: 0, min: 0, median: 1, max: 2, constant: false }),
		).toBeNull();
	});
});

describe('every machine name carries a plain gloss', () => {
	test('each metric leads with plain words and still shows the substrate field', () => {
		for (const m of METRICS) {
			expect(m.label.length).toBeGreaterThan(0);
			expect(m.gloss.length).toBeGreaterThan(0);
			expect(m.field).toMatch(/^[a-z_]+$/);
			// no-internal-vocabulary-is-load-bearing: the plain label may not BE the raw field.
			expect(m.label).not.toBe(m.field);
		}
	});

	test('the three the region hover card rendered are all present', () => {
		// displaced-structure-remains-reachable, at the module boundary: RegionHoverCard.svelte:17-19
		// rendered memberCount · salience · coherence, and this is where they are rehomed.
		const keys = METRICS.map((m) => m.key);

		expect(keys).toContain('member_count');
		expect(keys).toContain('salience');
		expect(keys).toContain('content_cohesion');
	});
});

describe('staleness is legible, never blocking', () => {
	test('a map that has never been materialized says so rather than reporting a date', () => {
		expect(
			describeStaleness({
				materialized_at: null,
				latest_touch: '2026-06-28T21:35:28Z',
				is_stale: true,
			}),
		).toBe('This shape has never been worked out.');
	});

	test('a settled shape reports when it was worked out', () => {
		const out = describeStaleness({
			materialized_at: '2026-08-14T14:03:07.844022Z',
			latest_touch: '2026-08-14T14:03:07.844022Z',
			is_stale: false,
		});

		expect(out).toContain('worked out');
		expect(out).not.toContain('stale');
	});

	test('a stale shape says the work moved on, and does not read as an error', () => {
		const out = describeStaleness({
			materialized_at: '2026-08-14T14:03:07.844022Z',
			latest_touch: '2026-08-19T10:00:00Z',
			is_stale: true,
		});

		expect(out).toContain('changed since');
		expect(out).not.toMatch(/error|failed|invalid/i);
	});
});

describe('a metric with one distinct value is said once, not tabulated 501 times', () => {
	const build = (...cent: (number | null)[]) =>
		analyseShape(
			cent.map((_, i) => shapeRow({ region_id: `00000000-0000-0000-0000-00000000000${i}` })),
			cent.map((c, i) =>
				metricRow({ region_id: `00000000-0000-0000-0000-00000000000${i}`, centrality: c }),
			),
		).regions;

	test('a varying metric gets a column', () => {
		const r = reportMetrics(build(1, 2, 3)).find((m) => m.spec.key === 'centrality')!;
		expect(r.asColumn).toBe(true);
	});

	test('a constant metric does not', () => {
		const r = reportMetrics(build(0, 0, 0)).find((m) => m.spec.key === 'centrality')!;
		expect(r.asColumn).toBe(false);
		expect(r.distribution.constant).toBe(true);
	});

	test('a metric nothing computed does not either, and is not mistaken for constant', () => {
		const r = reportMetrics(build(null, null)).find((m) => m.spec.key === 'centrality')!;
		expect(r.asColumn).toBe(false);
		expect(r.distribution.constant).toBe(false);
		expect(r.distribution.n).toBe(0);
	});

	test('every metric is reported, column or not — none is silently dropped', () => {
		expect(reportMetrics(build(0, 0))).toHaveLength(METRICS.length);
	});
});

describe('what a context genuinely does not have is declared, not fabricated', () => {
	test('the absence names what a context is, rather than reporting a failed lookup', () => {
		expect(CONTEXT_HAS_NO_MAP_READOUT).toContain('a context has neither');
		expect(CONTEXT_HAS_NO_MAP_READOUT).not.toMatch(/error|failed|missing|not found/i);
	});

	test('an empty regulation set is a fact about the map, not a zero', () => {
		expect(describeRegulation(0)).toBe('No concepts have been set to regulate this map.');
		expect(describeRegulation(1)).toContain('1 concept regulates');
	});

	test('the grouping count says whose order it is showing', () => {
		expect(describeGroupingCount(406)).toBe(
			'406 groupings, in the order this place itself ranks them.',
		);
		expect(describeGroupingCount(0)).toBe('This place has no groupings yet.');
	});
});
