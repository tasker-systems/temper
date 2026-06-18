import { describe, expect, it } from 'vitest';
import { decoratedRef, parseRef } from './ref';

const UUID = '0190f3a1-7c2b-7e44-9a1f-2b3c4d5e6f70';

describe('parseRef / decoratedRef', () => {
	it('round-trips a decorated ref back to its uuid', () => {
		const ref = decoratedRef('my-cool-doc', UUID);
		expect(ref).toBe(`my-cool-doc-${UUID}`);
		expect(parseRef(ref)).toBe(UUID);
	});

	it('passes a bare uuid through unchanged', () => {
		expect(parseRef(UUID)).toBe(UUID);
	});

	it('ignores a slug half that itself contains hyphens', () => {
		const ref = decoratedRef('a-slug-with-many-hyphens', UUID);
		expect(parseRef(ref)).toBe(UUID);
	});

	it('falls back to the bare id when no slug is available', () => {
		expect(decoratedRef(null, UUID)).toBe(UUID);
		expect(decoratedRef(undefined, UUID)).toBe(UUID);
		expect(decoratedRef('', UUID)).toBe(UUID);
	});
});
