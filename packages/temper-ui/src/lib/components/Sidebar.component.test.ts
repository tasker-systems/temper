import { fireEvent, render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { themeStore } from '$lib/stores/theme.svelte';
import { resetAppContext } from '../../test/app-context';
import Sidebar from './Sidebar.svelte';

vi.mock('$app/stores', () => import('../../test/app-context'));

beforeEach(() => {
	resetAppContext();
	window.matchMedia ??= ((query: string) =>
		({
			matches: query.includes('dark'),
			addEventListener: () => {},
		}) as unknown as MediaQueryList) as typeof window.matchMedia;
	themeStore.init();
});

const mount = (collapsed: boolean) =>
	render(Sidebar, {
		contexts: [],
		teams: [],
		selfProfileId: 'self',
		user: { display_name: 'Reader', email: 'reader@example.com' },
		collapsed,
		onToggle: () => {},
	});

describe('the sidebar head carries the chrome menu', () => {
	it.each([
		['expanded', false],
		['collapsed', true],
	])('reaches every appearance choice and Sign out with the rail %s', async (_, collapsed) => {
		const { getByRole, getByTestId } = mount(collapsed);

		await fireEvent.click(getByRole('button', { name: 'Menu' }));

		const menu = getByTestId('chrome-menu');
		expect(
			[...menu.querySelectorAll('[data-testid="theme-switch"] button')].map((b) =>
				b.textContent?.trim(),
			),
		).toEqual(['Follow system', 'Quiet Instrument', 'Quiet Instrument — Paper']);
		expect(menu.querySelector('a[href="/auth/logout"]')?.textContent?.trim()).toBe('Sign out');
		expect(menu.textContent).toContain('Reader');
	});

	it.each([
		['expanded', false],
		['collapsed', true],
	])(
		'renders no appearance control or Sign out outside the menu with the rail %s',
		(_, collapsed) => {
			const { queryByTestId, container } = mount(collapsed);
			expect(queryByTestId('theme-switch')).toBeNull();
			expect(container.querySelector('a[href="/auth/logout"]')).toBeNull();
		},
	);
});
