import { describe, expect, it } from 'vitest';
import { ARRIVAL } from '$lib/first-view';
import { HOSTILE_RETURN_TO } from './hostile-return-to.fixture';
import { safeReturnTo } from './return-to';

const ORIGIN = 'https://temper.test';

describe('safeReturnTo honours only a path on this site', () => {
	it.each([
		['/graph/@me', '/graph/@me'],
		['/vault', '/vault'],
		['/request-access', '/request-access'],
		['/vault?tasks=24&goals=16', '/vault?tasks=24&goals=16'],
		['/graph/%40me?q=a%20b', '/graph/%40me?q=a%20b'],
		['/resource/x#section', '/resource/x#section'],
	])('keeps the site path %s', (value, expected) => {
		expect(safeReturnTo(value, ORIGIN)).toBe(expected);
	});

	it('falls back to the first view when nothing is named', () => {
		expect(safeReturnTo(null, ORIGIN)).toBe(ARRIVAL);
		expect(safeReturnTo(undefined, ORIGIN)).toBe(ARRIVAL);
	});

	it.each(HOSTILE_RETURN_TO)('refuses %s', (_label, value) => {
		expect(safeReturnTo(value, ORIGIN)).toBe(ARRIVAL);
	});
});
