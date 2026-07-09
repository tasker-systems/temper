import { describe, expect, it } from 'vitest';
import { trayModel } from './residualTray';

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
		expect(cells[1].width).toBeGreaterThanOrEqual(118);
		expect(cells[0].x).toBe(0);
		expect(cells[1].x).toBe(cells[0].width);
	});

	it('orders by count descending regardless of input order', () => {
		const cells = trayModel(
			[{ value: 'decision', count: 9 }, { value: 'session', count: 395 }],
			900
		);
		expect(cells.map((c) => c.value)).toEqual(['session', 'decision']);
	});
});
