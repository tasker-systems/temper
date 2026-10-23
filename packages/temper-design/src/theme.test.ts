import { describe, expect, it } from 'vitest';
import { coercePreference, DEFAULT_PREFERENCE, resolveTheme, THEMES } from './theme.js';

describe('theme discovery', () => {
	it('finds every theme the design package indexes, paired as counterparts', () => {
		const names = THEMES.map((t) => t.name);
		expect(names).toContain('quiet-instrument');
		expect(names).toContain('quiet-instrument-paper');
		for (const t of THEMES) {
			if (!t.counterpart) continue;
			const other = THEMES.find((o) => o.name === t.counterpart);
			expect(other?.counterpart).toBe(t.name);
		}
	});
});

describe('resolveTheme', () => {
	it('follows the system within a family', () => {
		const pref = { follow: 'system', family: 'quiet-instrument' } as const;
		expect(resolveTheme(pref, true)).toBe('quiet-instrument');
		expect(resolveTheme(pref, false)).toBe('quiet-instrument-paper');
		const paper = { follow: 'system', family: 'quiet-instrument-paper' } as const;
		expect(resolveTheme(paper, true)).toBe('quiet-instrument');
	});
	it('holds a fixed theme regardless of the system', () => {
		expect(resolveTheme({ follow: 'fixed', name: 'quiet-instrument-paper' }, true)).toBe(
			'quiet-instrument-paper',
		);
	});
	it('never returns a theme that does not exist', () => {
		expect(resolveTheme({ follow: 'fixed', name: 'neon' }, true)).toBe('quiet-instrument');
		expect(resolveTheme({ follow: 'system', family: 'neon' }, false)).toBe(
			'quiet-instrument-paper',
		);
	});
});

describe('coercePreference', () => {
	it('falls back to the default on absent or malformed values', () => {
		expect(coercePreference(undefined)).toEqual(DEFAULT_PREFERENCE);
		expect(coercePreference(null)).toEqual(DEFAULT_PREFERENCE);
		expect(coercePreference('quiet-instrument')).toEqual(DEFAULT_PREFERENCE);
		expect(coercePreference({ follow: 'sideways' })).toEqual(DEFAULT_PREFERENCE);
		expect(coercePreference({ follow: 'fixed' })).toEqual(DEFAULT_PREFERENCE);
	});
	it('keeps a recognisable preference whole', () => {
		const stored = { follow: 'fixed', name: 'quiet-instrument-paper' } as const;
		expect(coercePreference(stored)).toEqual(stored);
	});
});
