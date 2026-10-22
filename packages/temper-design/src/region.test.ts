import { describe, expect, it } from 'vitest';
import { REGION_STATES, regionWords } from './region.js';

/**
 * `region-states-read-alike` asks for four distinct signals, carried on more than colour. Colour
 * and border are the recipe's (`contract/region.css`); the words and the marker are this
 * module's, and each must tell the four apart on its own.
 */
describe('regionWords', () => {
	const all = REGION_STATES.map((state) => regionWords(state, 'history'));

	it('gives each state its own marker', () => {
		expect(new Set(all.map((w) => w.marker)).size).toBe(REGION_STATES.length);
	});

	it('gives each state its own sentence', () => {
		expect(new Set(all.map((w) => w.words)).size).toBe(REGION_STATES.length);
	});

	it('composes the label into every sentence', () => {
		for (const w of all) expect(w.words.toLowerCase()).toContain('history');
	});

	it('names what a failure failed to read, and that nothing was read', () => {
		const failed = regionWords('failed', 'history');
		expect(failed.words).toBe('History unavailable');
		expect(failed.qualifier).toContain('nothing here was read');
	});

	it('names what a give-up stopped waiting for, never the mechanism', () => {
		const gaveUp = regionWords('gave-up', 'history');
		expect(gaveUp.words).toBe('We stopped waiting for history');
		expect(`${gaveUp.words}${gaveUp.qualifier}`.toLowerCase()).not.toContain('timeout');
	});

	it('announces arrival and give-up politely, failure as an alert, and empty not at all', () => {
		expect(Object.fromEntries(REGION_STATES.map((s) => [s, regionWords(s, 'x').role]))).toEqual({
			arriving: 'status',
			empty: null,
			'gave-up': 'status',
			failed: 'alert',
		});
	});
});
