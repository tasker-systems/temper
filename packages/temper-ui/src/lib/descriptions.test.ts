import { describe, expect, it } from 'vitest';
import { editableKind, revisedValue } from './descriptions';

describe('editableKind', () => {
	it('names the single-value types this surface can round-trip', () => {
		expect(editableKind('high')).toBe('string');
		expect(editableKind('')).toBe('string');
		expect(editableKind(3)).toBe('number');
		expect(editableKind(0)).toBe('number');
		expect(editableKind(false)).toBe('boolean');
	});

	it('offers nothing for a structured value', () => {
		// The register's exclusion, and the one most likely to be met immediately: `tags`,
		// `keywords` and `relates_to` all hold lists. A reader sees one and cannot change it
		// here — recorded as excluded rather than left to look like a bug.
		expect(editableKind(['docs', 'ui'])).toBeNull();
		expect(editableKind({ nested: 1 })).toBeNull();
		expect(editableKind(null)).toBeNull();
		expect(editableKind(undefined)).toBeNull();
	});

	it('offers nothing for a number that cannot survive a round trip', () => {
		expect(editableKind(Number.NaN)).toBeNull();
		expect(editableKind(Number.POSITIVE_INFINITY)).toBeNull();
	});
});

describe('revisedValue', () => {
	it('keeps a number a number', () => {
		// THE BITE. A form submits text, so without this `priority: 3` revised to `4` stores
		// `"4"` — a type change nobody asked for, which the table cannot show (both render as
		// `4`) and which every downstream consumer sees.
		expect(revisedValue('4', 'number')).toBe(4);
		expect(revisedValue('-2.5', 'number')).toBe(-2.5);
	});

	it('keeps a boolean a boolean', () => {
		expect(revisedValue('true', 'boolean')).toBe(true);
		expect(revisedValue('false', 'boolean')).toBe(false);
	});

	it('lets a reader retype a number as words rather than refusing them', () => {
		// Changing what a description *is* is a legitimate revision, not an error. The old type
		// is a default, never a constraint the reader has to satisfy.
		expect(revisedValue('three', 'number')).toBe('three');
		expect(revisedValue('', 'number')).toBe('');
		expect(revisedValue('yes', 'boolean')).toBe('yes');
	});

	it('leaves text as text', () => {
		expect(revisedValue('high', 'string')).toBe('high');
		expect(revisedValue('4', 'string')).toBe('4');
	});
});
