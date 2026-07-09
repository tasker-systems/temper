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
 * Below this a cell cannot hold its label + count legibly. When the buckets are so many
 * that even the floor cannot fit them all, the rail scrolls horizontally (never the page).
 * See ResidualTray.svelte's `overflow-x: auto`.
 */
export const MIN_CELL = 118;

/** The flex `gap` between cells, and the rail's horizontal padding, in ResidualTray.svelte.
 *  The model must reserve them: cells summing to exactly `width` still overflow a rail that
 *  renders gutters between them. */
const CELL_GAP = 6;
const RAIL_PAD_X = 4;

/**
 * Lay the buckets out as a rail of cells that **fits** `width` whenever it can.
 *
 * Every cell gets `MIN_CELL` so its label and count stay legible, then the surplus is shared
 * by count — so the dominant bucket still reads as dominant without pushing the tail off the
 * edge. Sizing purely by share and clamping the small cells up overflows by construction: the
 * clamped-up pixels are never taken back from the large cell, so a 395/24/11/4 split spends
 * 1519px of a 1280px rail and the last doorway is silently cut off.
 *
 * Only when `MIN_CELL` × N exceeds the available width does the rail scroll — the one case
 * where no layout can show every bucket at a legible size.
 */
export function trayModel(buckets: ResidualBucket[], width: number): TrayCell[] {
	if (buckets.length === 0) return [];
	const sorted = [...buckets].sort((a, b) => b.count - a.count);
	const total = sorted.reduce((a, b) => a + b.count, 0);

	const gutters = CELL_GAP * (sorted.length - 1) + RAIL_PAD_X;
	const available = width - gutters;
	const surplus = available - MIN_CELL * sorted.length;

	let x = 0;
	return sorted.map((b) => {
		// surplus <= 0: every bucket sits at the floor and the rail scrolls.
		const share = surplus > 0 && total > 0 ? (b.count / total) * surplus : 0;
		const w = MIN_CELL + share;
		const cell = { value: b.value, count: b.count, x, width: w };
		x += w + CELL_GAP;
		return cell;
	});
}
