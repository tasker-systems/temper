import { describe, expect, test } from 'vitest';
import {
	CANVAS_FLOOR_PX,
	RAIL_PX,
	READOUT_PX,
	readoutMustYield,
	YIELD_BELOW_PX,
} from './instrument';

/**
 * The floor, as a decision rather than as a rendering.
 *
 * `src/test/README.md`: *"No test here may claim a thing is legible, readable, or correctly
 * sized."* jsdom computes no layout, so the component test beside `GraphPage.svelte` can only
 * witness **which branch rendered**. What the number means is asserted here, where it is a value.
 */
const at = (surfaceWidth: number, over: Partial<Parameters<typeof readoutMustYield>[0]> = {}) =>
	readoutMustYield({ surfaceWidth, railOpen: true, readoutPresent: true, ...over });

describe('the canvas has a floor, and the readout is what yields to it', () => {
	test('the threshold is the floor plus the two fixed things beside it, not a written-down number', () => {
		// The whole reason the arithmetic lives in TypeScript: a container-query condition cannot
		// read a custom property, so a CSS rule would have had to hard-code this sum and the floor
		// would then exist in two places. Change `CANVAS_FLOOR_PX` and this moves with it.
		expect(YIELD_BELOW_PX).toBe(CANVAS_FLOOR_PX + RAIL_PX + READOUT_PX);
	});

	test('above the threshold nothing yields — this is a floor, not a redesign', () => {
		expect(at(YIELD_BELOW_PX)).toBe(false);
		expect(at(YIELD_BELOW_PX + 1)).toBe(false);
		expect(at(2000)).toBe(false);
	});

	test('one pixel below it, the readout yields', () => {
		expect(at(YIELD_BELOW_PX - 1)).toBe(true);
	});

	test('at the width the ruling calls a smudge, it yields', () => {
		// `[ruled — 2026-08-22, Pete]` *"At 1280px it is ~610px and a 130-node force layout is a
		// smudge."* That screen is the reason this exists, so it is named rather than implied.
		expect(at(1280)).toBe(true);
		expect(1280 - RAIL_PX - READOUT_PX).toBeLessThan(CANVAS_FLOOR_PX);
	});

	test('at the width it reports without complaint, it does not', () => {
		// *"At 1440px it is ~770px."* Reported neutrally, so the floor must sit under it.
		expect(at(1440)).toBe(false);
	});

	test('with no rail open there is nothing to yield to, at any width', () => {
		// The canvas already has everything but the readout. The ruling orders three things
		// competing for one row; with two, there is no competition to resolve.
		expect(at(400, { railOpen: false })).toBe(false);
		expect(at(YIELD_BELOW_PX - 1, { railOpen: false })).toBe(false);
	});

	test('with no readout in the second track there is nothing to collapse', () => {
		expect(at(400, { readoutPresent: false })).toBe(false);
	});

	test('an unmeasured surface yields nothing — a zero is not a narrow screen', () => {
		// `0` is what the surface reports before its first observation and throughout SSR. Read as
		// a width it is narrower than anything, and every reader would be served a collapsed panel
		// by the server and then watch it open.
		expect(at(0)).toBe(false);
		expect(at(-1)).toBe(false);
	});
});
