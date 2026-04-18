import type { Viewport } from './positions';

/** Scalar tuning values for a d3-force simulation. */
export interface ForceConfig {
	/** `forceManyBody().strength()` — negative = repulsion. */
	charge: number;
	/** `forceLink().distance()` — target edge length in px. */
	linkDistance: number;
	/** `forceCollide().radius()` padding beyond node radius. */
	collisionPadding: number;
	/** `forceCenter(x, y)` — usually viewport midpoint. */
	centerX: number;
	/** `forceCenter(x, y)`. */
	centerY: number;
}

/** Compute force-simulation tunings for a given viewport size. */
export function buildForceConfig(viewport: Viewport): ForceConfig {
	return {
		charge: -250,
		linkDistance: 60,
		collisionPadding: 2,
		centerX: viewport.width / 2,
		centerY: viewport.height / 2
	};
}
