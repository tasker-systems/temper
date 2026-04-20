/**
 * Zoom-tier classification for label culling. Three named tiers gate which
 * nodes render labels (and which extra metadata like date-strips appears),
 * per kg-handoff.md § PR 5 — *Zoom tiers & label culling*.
 *
 *   - `overview` (`< 0.5`): only aggregator labels. Participants render as
 *     short colored tick marks so the clustering reads at a glance.
 *   - `mid`      (`0.5 – 1.2`): steady state — all labels on, full typography.
 *   - `detail`   (`> 1.2`): labels plus date strips (and, later, task stage
 *     tags — that data source is stubbed for now).
 */

export type ZoomTier = 'overview' | 'mid' | 'detail';

export const ZOOM_THRESHOLDS = {
	/** Below this zoom we're in `overview`. */
	overview: 0.5,
	/** Above this zoom we're in `detail`. */
	detail: 1.2
} as const;

/**
 * Pure threshold classifier — same zoom level always yields the same tier,
 * so the zoom handler can cache the last value and only toggle classes on
 * an actual tier change (which is the anti-jitter strategy).
 *
 * Boundary behavior: values exactly at `ZOOM_THRESHOLDS.overview` or
 * `ZOOM_THRESHOLDS.detail` resolve to `mid`. A continuous wheel scroll
 * passing through the boundary fires exactly one transition.
 */
export function zoomTier(zoom: number): ZoomTier {
	if (zoom < ZOOM_THRESHOLDS.overview) return 'overview';
	if (zoom > ZOOM_THRESHOLDS.detail) return 'detail';
	return 'mid';
}

/** Cytoscape class name for a given tier — e.g. `"tier-overview"`. */
export function tierClass(tier: ZoomTier): string {
	return `tier-${tier}`;
}

/** All tier class names — useful for `.removeClass(ALL_TIER_CLASSES)`. */
export const ALL_TIER_CLASSES = 'tier-overview tier-mid tier-detail';
