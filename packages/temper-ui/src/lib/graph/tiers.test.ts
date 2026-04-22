import { describe, expect, it } from 'vitest';
import {
	ALL_TIER_CLASSES,
	MAX_GRAPH_ZOOM,
	MIN_GRAPH_ZOOM,
	tierClass,
	zoomTier,
	ZOOM_THRESHOLDS
} from './tiers';

describe('zoomTier thresholds', () => {
	it('matches the kg-handoff.md prototype values', () => {
		expect(ZOOM_THRESHOLDS.overview).toBe(0.5);
		expect(ZOOM_THRESHOLDS.detail).toBe(1.2);
	});
});

describe('zoomTier', () => {
	it('returns "overview" below the overview threshold', () => {
		expect(zoomTier(0.25)).toBe('overview');
		expect(zoomTier(0.49)).toBe('overview');
	});

	it('returns "mid" inside the mid band — inclusive at boundaries', () => {
		// Exact boundary values land in "mid" so a continuous scroll through the
		// threshold fires exactly one transition.
		expect(zoomTier(0.5)).toBe('mid');
		expect(zoomTier(1)).toBe('mid');
		expect(zoomTier(1.2)).toBe('mid');
	});

	it('returns "detail" above the detail threshold', () => {
		expect(zoomTier(1.21)).toBe('detail');
		expect(zoomTier(2.5)).toBe('detail');
	});

	it('is deterministic — equal input yields equal output (no-op caching strategy)', () => {
		for (const z of [0.1, 0.5, 0.9, 1.2, 2.0]) {
			expect(zoomTier(z)).toBe(zoomTier(z));
		}
	});
});

describe('tierClass', () => {
	it('prefixes the tier name with "tier-"', () => {
		expect(tierClass('overview')).toBe('tier-overview');
		expect(tierClass('mid')).toBe('tier-mid');
		expect(tierClass('detail')).toBe('tier-detail');
	});
});

describe('ALL_TIER_CLASSES', () => {
	it('lists every tier class for bulk .removeClass()', () => {
		const parts = ALL_TIER_CLASSES.split(' ');
		expect(new Set(parts)).toEqual(new Set(['tier-overview', 'tier-mid', 'tier-detail']));
	});
});

describe('graph zoom bounds', () => {
	// Regression guard for the "empty canvas" bug: if `minZoom` drops below the
	// overview threshold, `cy.fit()` on a wide bbox lands in the overview tier
	// where participants render as 12×3 ticks with `text-opacity: 0`, making
	// the graph look empty on the near-black canvas.
	it('pins MIN_GRAPH_ZOOM to the overview threshold so fit() never clamps into overview', () => {
		expect(MIN_GRAPH_ZOOM).toBe(ZOOM_THRESHOLDS.overview);
		expect(zoomTier(MIN_GRAPH_ZOOM)).toBe('mid');
	});

	it('keeps MAX_GRAPH_ZOOM above the detail threshold so detail tier is reachable', () => {
		expect(MAX_GRAPH_ZOOM).toBeGreaterThan(ZOOM_THRESHOLDS.detail);
	});
});
