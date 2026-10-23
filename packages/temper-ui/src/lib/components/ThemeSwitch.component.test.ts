import { fireEvent, render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readCookie, themeStore } from '$lib/stores/theme.svelte';
import { resetAppContext } from '../../test/app-context';
import ThemeSwitch from './ThemeSwitch.svelte';

vi.mock('$app/stores', () => import('../../test/app-context'));

const root = () => document.documentElement;

beforeEach(() => {
	resetAppContext();
	// biome-ignore lint/suspicious/noDocumentCookie: the store under test writes document.cookie
	document.cookie = 'temper.theme=; Path=/; Max-Age=0';
	root().setAttribute('data-theme', 'quiet-instrument');
	root().setAttribute('data-theme-light', 'quiet-instrument-paper');
	window.matchMedia ??= ((query: string) =>
		({
			matches: query.includes('dark'),
			addEventListener: () => {},
		}) as unknown as MediaQueryList) as typeof window.matchMedia;
	themeStore.init();
});

describe('the appearance control', () => {
	it('offers following the system and every published theme, by title', () => {
		const { getAllByRole } = render(ThemeSwitch);
		expect(getAllByRole('button').map((b) => b.textContent?.trim())).toEqual([
			'Follow system',
			'Quiet Instrument',
			'Quiet Instrument — Paper',
		]);
	});

	it('choosing a theme repaints the root and keeps the choice in the cookie', async () => {
		const { getByRole } = render(ThemeSwitch);
		await fireEvent.click(getByRole('button', { name: 'Quiet Instrument — Paper' }));
		expect(root().getAttribute('data-theme')).toBe('quiet-instrument-paper');
		expect(readCookie(document.cookie, 'temper.theme')).toBe('fixed:quiet-instrument-paper');
		expect(
			getByRole('button', { name: 'Quiet Instrument — Paper' }).getAttribute('aria-pressed'),
		).toBe('true');
	});

	it('the store owns the root once it starts: the first-paint candidate is gone', () => {
		expect(root().hasAttribute('data-theme-light')).toBe(false);
	});
});

describe('readCookie', () => {
	it('finds one cookie among several, and nothing when absent', () => {
		expect(readCookie('a=1; temper.theme=fixed%3Aquiet-instrument; b=2', 'temper.theme')).toBe(
			'fixed:quiet-instrument',
		);
		expect(readCookie('a=1', 'temper.theme')).toBeNull();
	});

	/** The server's cookie parser tolerates a malformed escape; the client must too, or the
	 *  store throws on mount and the reader's theme is never applied. */
	it('reads a malformed escape as no cookie, rather than throwing', () => {
		expect(readCookie('temper.theme=%', 'temper.theme')).toBeNull();
		expect(readCookie('temper.theme=%E0%A4%A', 'temper.theme')).toBeNull();
	});
});
