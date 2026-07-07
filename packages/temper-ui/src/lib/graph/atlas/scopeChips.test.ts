import { describe, it, expect } from 'vitest';
import { deriveScopeChips } from './scopeChips';

describe('deriveScopeChips', () => {
	it('returns distinct owner_refs in stable (sorted) order', () => {
		expect(deriveScopeChips([{ owner_ref: '+tasker' }, { owner_ref: '@me' }, { owner_ref: '+tasker' }])).toEqual([
			'+tasker',
			'@me'
		]); // sorted: '+' < '@' by charCode
	});
	it('is empty for no bodies', () => {
		expect(deriveScopeChips([])).toEqual([]);
	});
});
