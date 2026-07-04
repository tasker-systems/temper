/**
 * Convex-hull outline for a region/neighborhood (spec C2-D5/D6). d3-polygon
 * computes the hull; we emit a padded closed SVG path. Pure.
 */
import { polygonHull } from 'd3-polygon';

export function hullPath(points: [number, number][], padding = 0): string | null {
	if (points.length < 3) return null;
	const hull = polygonHull(points);
	if (!hull) return null;

	const cx = hull.reduce((s, p) => s + p[0], 0) / hull.length;
	const cy = hull.reduce((s, p) => s + p[1], 0) / hull.length;

	const expanded = hull.map(([x, y]) => {
		if (padding === 0) return [x, y];
		const dx = x - cx;
		const dy = y - cy;
		const len = Math.hypot(dx, dy) || 1;
		return [x + (dx / len) * padding, y + (dy / len) * padding];
	});

	return `M${expanded.map(([x, y]) => `${x.toFixed(2)},${y.toFixed(2)}`).join('L')}Z`;
}
