import { DEFAULT_PREFERENCE } from '@tasker-systems/temper-design/theme';
import { describe, expect, it } from 'vitest';
import {
	decodePreference,
	encodePreference,
	isMarketingRoute,
	pageTheme,
	withPageTheme,
} from './theme';

describe('the preference cookie', () => {
	it('round-trips both kinds of preference', () => {
		const system = { follow: 'system', family: 'quiet-instrument' } as const;
		const fixed = { follow: 'fixed', name: 'quiet-instrument-paper' } as const;
		expect(decodePreference(encodePreference(system))).toEqual(system);
		expect(decodePreference(encodePreference(fixed))).toEqual(fixed);
	});

	it('reads an absent or unrecognisable cookie as the default', () => {
		expect(decodePreference(undefined)).toEqual(DEFAULT_PREFERENCE);
		expect(decodePreference('')).toEqual(DEFAULT_PREFERENCE);
		expect(decodePreference('quiet-instrument-paper')).toEqual(DEFAULT_PREFERENCE);
		expect(decodePreference('sideways:quiet-instrument')).toEqual(DEFAULT_PREFERENCE);
	});
});

describe('the theme a page is served in', () => {
	it('serves a fixed theme as it is, with no candidate for the script', () => {
		expect(pageTheme({ follow: 'fixed', name: 'quiet-instrument-paper' }, false)).toEqual({
			theme: 'quiet-instrument-paper',
			light: null,
		});
	});

	it('serves a system preference dark, carrying the light counterpart for the script', () => {
		expect(pageTheme({ follow: 'system', family: 'quiet-instrument' }, false)).toEqual({
			theme: 'quiet-instrument',
			light: 'quiet-instrument-paper',
		});
	});

	it('serves marketing in Quiet Instrument whatever the reader prefers', () => {
		expect(pageTheme({ follow: 'fixed', name: 'quiet-instrument-paper' }, true)).toEqual({
			theme: 'quiet-instrument',
			light: null,
		});
		expect(pageTheme({ follow: 'system', family: 'quiet-instrument' }, true).light).toBeNull();
	});

	it('knows the marketing routes from the app routes', () => {
		expect(isMarketingRoute('/(public)')).toBe(true);
		expect(isMarketingRoute('/(public)/manifesto')).toBe(true);
		expect(isMarketingRoute('/request-access')).toBe(true);
		expect(isMarketingRoute('/(app)/vault/all')).toBe(false);
		expect(isMarketingRoute(null)).toBe(false);
	});
});

describe('writing the theme onto the document', () => {
	const shell = '<html lang="en" data-theme="quiet-instrument">';

	it('rewrites the root attribute', () => {
		expect(withPageTheme(shell, { theme: 'quiet-instrument-paper', light: null })).toBe(
			'<html lang="en" data-theme="quiet-instrument-paper">',
		);
	});

	it('adds the light candidate when there is one', () => {
		expect(
			withPageTheme(shell, { theme: 'quiet-instrument', light: 'quiet-instrument-paper' }),
		).toBe(
			'<html lang="en" data-theme="quiet-instrument" data-theme-light="quiet-instrument-paper">',
		);
	});
});
