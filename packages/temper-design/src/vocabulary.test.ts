import { describe, expect, it } from 'vitest';
import { bindingsFrom, CORE_BINDINGS, SLOTS, tintFor, tintVar } from './vocabulary';

describe("core's doc types on the categorical slots", () => {
	it('binds every core doc type to its ruled slot', () => {
		expect(Object.fromEntries(CORE_BINDINGS.map((b) => [b.doctype, b.tint]))).toEqual({
			task: 'cat-2',
			goal: 'cat-5',
			commitment: 'cat-5',
			session: 'cat-3',
			memory: 'cat-3',
			research: 'cat-7',
			question: 'cat-7',
			decision: 'cat-4',
			principle: 'cat-4',
			concept: 'cat-6',
			domain: 'cat-6',
			theme: 'cat-6',
			fact: 'cat-8',
			concern: 'cat-1',
		});
	});

	it('resolves a bound doc type to its slot, and anything else to the neutral', () => {
		expect(tintFor('task')).toBe('cat-2');
		expect(tintFor('theme')).toBe('cat-6');
		expect(tintFor('story')).toBe('cat-neutral');
		expect(tintFor('')).toBe('cat-neutral');
		expect(tintFor(null)).toBe('cat-neutral');
		expect(tintFor(undefined)).toBe('cat-neutral');
		// An inherited key is not a binding.
		expect(tintFor('constructor')).toBe('cat-neutral');
	});

	it('names the role a tint paints with', () => {
		expect(tintVar('cat-3')).toBe('var(--tp-cat-3)');
		expect(tintVar('cat-neutral')).toBe('var(--tp-cat-neutral)');
	});
});

describe('a vocabulary binding', () => {
	it('resolves through the bindings it is given', () => {
		const plugin = bindingsFrom([{ doctype: 'story', tint: 'cat-1' }]);
		expect(tintFor('story', plugin)).toBe('cat-1');
		expect(tintFor('task', plugin)).toBe('cat-neutral');
	});

	it.each([
		['the neutral', 'cat-neutral'],
		['a chrome role', 'accent'],
		['a condition', 'danger'],
		['a slot that does not exist', 'cat-9'],
		['a colour', '#e8942e'],
	])('refuses a binding to %s', (_, tint) => {
		expect(() => bindingsFrom([{ doctype: 'story', tint }])).toThrow(/slot/);
	});

	it('refuses a doc type bound twice', () => {
		expect(() =>
			bindingsFrom([
				{ doctype: 'story', tint: 'cat-1' },
				{ doctype: 'story', tint: 'cat-2' },
			]),
		).toThrow(/twice/);
	});

	it('owns eight slots', () => {
		expect(SLOTS).toEqual(['cat-1', 'cat-2', 'cat-3', 'cat-4', 'cat-5', 'cat-6', 'cat-7', 'cat-8']);
	});
});
