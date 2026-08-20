import { describe, expect, test } from 'vitest';
import { jsonBody } from './json-body';

describe('jsonBody', () => {
	test('a bigint becomes a JSON number, where a bare JSON.stringify throws', () => {
		const body = { terms: { regions: BigInt(3), limit: BigInt(50) } };

		expect(() => JSON.stringify(body)).toThrow();
		expect(JSON.parse(jsonBody(body))).toEqual({ terms: { regions: 3, limit: 50 } });
	});

	test('a bigint beyond safe-integer range throws rather than rounding', () => {
		// A silently-rounded id is the one outcome nobody would notice, so it is the one refused.
		const unsafe = BigInt(Number.MAX_SAFE_INTEGER) + BigInt(2);

		expect(() => jsonBody({ n: unsafe })).toThrow(RangeError);
		expect(() => jsonBody({ n: -unsafe })).toThrow(RangeError);
		expect(JSON.parse(jsonBody({ n: BigInt(Number.MAX_SAFE_INTEGER) }))).toEqual({
			n: Number.MAX_SAFE_INTEGER,
		});
	});

	test('everything else encodes exactly as JSON.stringify would', () => {
		const plain = { a: 1, b: 'two', c: [null, true], d: { e: 0.5 } };

		expect(jsonBody(plain)).toBe(JSON.stringify(plain));
	});

	test('a nested bigint is reached, not only a top-level one', () => {
		const nested = { stages: [{ name: 's1', terms: { regions: BigInt(3) } }] };

		expect(JSON.parse(jsonBody(nested)).stages[0].terms.regions).toBe(3);
	});
});
