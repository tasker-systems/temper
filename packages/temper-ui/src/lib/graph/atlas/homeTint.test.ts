import { describe, expect, test } from 'vitest';
import { intensityFor, buildTint, researchTint } from './homeTint';

describe('intensityFor', () => {
	test('a zero member count floors at 0.3', () => {
		expect(intensityFor(0, 10)).toBe(0.3);
	});

	test('a count equal to max reaches the 0.9 ceiling', () => {
		expect(intensityFor(10, 10)).toBeCloseTo(0.9);
	});

	test('a negative count clamps to the 0.3 floor', () => {
		expect(intensityFor(-5, 10)).toBe(0.3);
	});
});

describe('buildTint', () => {
	test('@me anchors at the exact base cool-blue', () => {
		expect(buildTint('@me')).toBe('hsl(200 44% 62%)');
	});

	test('a +team lands in the cool blue→indigo band', () => {
		const hue = Number(buildTint('+storyteller').match(/hsl\((\d+) /)?.[1]);
		expect(hue).toBeGreaterThanOrEqual(200);
		expect(hue).toBeLessThanOrEqual(200 + 7 * 8);
	});

	test('a +team tint is deterministic across calls', () => {
		expect(buildTint('+storyteller')).toBe(buildTint('+storyteller'));
	});

	test('two different teams can produce different tints', () => {
		expect(buildTint('+alpha')).not.toBe(buildTint('+bravo'));
	});
});

describe('researchTint', () => {
	test('a non-+ scope anchors at the base cogmap-orange', () => {
		expect(researchTint('temper')).toBe('hsl(34 80% 56%)');
	});

	test('a +team lands in the warm red-orange→amber band', () => {
		const hue = Number(researchTint('+storyteller').match(/hsl\((\d+) /)?.[1]);
		expect(hue).toBeGreaterThanOrEqual(12);
		expect(hue).toBeLessThanOrEqual(12 + 7 * 6);
	});

	test('a +team tint is deterministic across calls', () => {
		expect(researchTint('+storyteller')).toBe(researchTint('+storyteller'));
	});
});
