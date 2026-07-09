import { describe, expect, it } from 'vitest';
import { MIN_CELL, trayModel } from './residualTray';

/** Total rail footprint: the cells plus the gutters the flex container renders between them. */
const railWidth = (cells: { width: number }[]) =>
	cells.reduce((a, c) => a + c.width, 0) + 6 * (cells.length - 1) + 4;

describe('trayModel', () => {
	it('is empty for a well-edged context — the tray vanishes', () => {
		expect(trayModel([], 900)).toEqual([]);
	});

	it('sizes cells by share but never below a legible minimum', () => {
		const cells = trayModel(
			[{ value: 'session', count: 395 }, { value: 'decision', count: 9 }],
			900
		);
		expect(cells).toHaveLength(2);
		expect(cells[0].width).toBeGreaterThan(cells[1].width);
		expect(cells[1].width).toBeGreaterThanOrEqual(MIN_CELL);
		expect(cells[0].x).toBe(0);
		expect(cells[1].x).toBe(cells[0].width + 6);
	});

	it('fits the rail when the floor allows it, so no doorway is cut off', () => {
		// The heavy-tailed real shape: sizing by share alone and clamping the tail up to
		// MIN_CELL spent 1519px of a 1280px rail, silently clipping the last bucket.
		const cells = trayModel(
			[
				{ value: 'session', count: 395 },
				{ value: 'decision', count: 24 },
				{ value: 'note', count: 11 },
				{ value: 'fact', count: 4 }
			],
			1280
		);
		expect(cells).toHaveLength(4);
		expect(railWidth(cells)).toBeLessThanOrEqual(1280);
		expect(cells.every((c) => c.width >= MIN_CELL)).toBe(true);
		expect(cells[0].width).toBeGreaterThan(cells[3].width * 3);
	});

	it('falls back to the legible floor — and lets the rail scroll — when too many buckets', () => {
		const many = Array.from({ length: 12 }, (_, i) => ({ value: `k${i}`, count: 12 - i }));
		const cells = trayModel(many, 600);
		expect(cells.every((c) => c.width === MIN_CELL)).toBe(true);
		expect(railWidth(cells)).toBeGreaterThan(600);
	});

	it('orders by count descending regardless of input order', () => {
		const cells = trayModel(
			[{ value: 'decision', count: 9 }, { value: 'session', count: 395 }],
			900
		);
		expect(cells.map((c) => c.value)).toEqual(['session', 'decision']);
	});
});
