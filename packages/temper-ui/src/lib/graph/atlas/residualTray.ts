// residualTray.ts
/**
 * The residual tray: a doorway to what reaches no container, NOT a landmark of the work.
 *
 * Residuals are deliberately kept OUT of the force field. In-field they capture the label
 * gate (4 of 10 on real data) and, worse, `intensityOf` normalizes against the largest
 * weight in the field — so one 395-item bucket drags every real goal's intensity toward
 * zero (Maintenance 1.00 → 0.16). They do not merely steal labels; they flatten the survey.
 *
 * On well-edged data containers absorb their members and this model returns `[]`, so the
 * tray disappears with no special case.
 */
import type { ResidualBucket } from '$lib/types/generated/graph_context';

export interface TrayCell {
	value: string;
	count: number;
	x: number;
	width: number;
}

/**
 * Below this a cell cannot hold its label + count legibly. When many tiny buckets each
 * clamp to the floor, the total cell width can exceed `width` — the tray container then
 * scrolls horizontally (never the page). See ResidualTray.svelte's `overflow-x: auto`.
 */
export const MIN_CELL = 118;

export function trayModel(buckets: ResidualBucket[], width: number): TrayCell[] {
	if (buckets.length === 0) return [];
	const sorted = [...buckets].sort((a, b) => b.count - a.count);
	const total = sorted.reduce((a, b) => a + b.count, 0);
	let x = 0;
	return sorted.map((b) => {
		const w = Math.max(MIN_CELL, total > 0 ? (b.count / total) * width : MIN_CELL);
		const cell = { value: b.value, count: b.count, x, width: w };
		x += w;
		return cell;
	});
}
