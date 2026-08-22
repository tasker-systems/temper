import { describe, expect, it } from 'vitest';
import { regionStateFor } from './region';

/**
 * The client half of the give-up, and the reason it is a **value** rather than a class.
 *
 * `GaveUp` is thrown on the server and never reaches a browser: SvelteKit runs every rejected
 * streamed promise through `handleError` and serialises what that returns, so a prototype cannot
 * survive the trip. This function reads the field that does.
 */
describe('reading a give-up on the side of the boundary that has no classes', () => {
	it('a rejection carrying the give-up field is a give-up', () => {
		expect(regionStateFor({ message: 'Internal Error', gaveUp: 'history' })).toBe('gave-up');
	});

	it('a rejection that does not carry it is a plain failure', () => {
		expect(regionStateFor({ message: 'Internal Error' })).toBe('failed');
	});

	// The value is what discriminates, so anything that is not the value must not. An `Error`
	// instance is the shape a component test is tempted to construct, and it is exactly the shape
	// production never produces.
	it('a bare Error is a plain failure, whatever its name says', () => {
		const impostor = new Error('gave up waiting for history after 8000ms');
		impostor.name = 'GaveUp';
		expect(regionStateFor(impostor)).toBe('failed');
	});

	it('null and undefined are plain failures rather than a crash', () => {
		expect(regionStateFor(null)).toBe('failed');
		expect(regionStateFor(undefined)).toBe('failed');
	});
});
