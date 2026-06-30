import { describe, expect, it } from 'vitest';
import { storefrontEnabledFrom } from './storefront';

describe('storefrontEnabledFrom', () => {
	it('defaults to enabled when the env var is unset', () => {
		expect(storefrontEnabledFrom(undefined)).toBe(true);
	});

	it('stays enabled for any non-falsy value', () => {
		expect(storefrontEnabledFrom('true')).toBe(true);
		expect(storefrontEnabledFrom('1')).toBe(true);
		expect(storefrontEnabledFrom('on')).toBe(true);
		expect(storefrontEnabledFrom('yes')).toBe(true);
		expect(storefrontEnabledFrom('')).toBe(true);
	});

	it('disables on explicit falsy tokens (case- and whitespace-insensitive)', () => {
		expect(storefrontEnabledFrom('false')).toBe(false);
		expect(storefrontEnabledFrom('0')).toBe(false);
		expect(storefrontEnabledFrom('off')).toBe(false);
		expect(storefrontEnabledFrom('no')).toBe(false);
		expect(storefrontEnabledFrom('FALSE')).toBe(false);
		expect(storefrontEnabledFrom('  Off  ')).toBe(false);
	});
});
