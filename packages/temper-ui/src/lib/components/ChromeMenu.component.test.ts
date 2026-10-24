import { fireEvent, render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { themeStore } from '$lib/stores/theme.svelte';
import { resetAppContext } from '../../test/app-context';
import ChromeMenu from './ChromeMenu.svelte';

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

const mount = () =>
	render(ChromeMenu, { user: { display_name: 'Reader', email: 'r@example.com' } });

describe('the chrome menu', () => {
	it('opens only when the reader opens it', async () => {
		const { getByRole, queryByTestId } = mount();
		const trigger = getByRole('button', { name: 'Menu' });
		expect(queryByTestId('chrome-menu')).toBeNull();
		expect(trigger.getAttribute('aria-expanded')).toBe('false');

		await fireEvent.click(trigger);

		expect(queryByTestId('chrome-menu')).not.toBeNull();
		expect(trigger.getAttribute('aria-expanded')).toBe('true');
	});

	it('closes on Escape', async () => {
		const { getByRole, queryByTestId } = mount();
		await fireEvent.click(getByRole('button', { name: 'Menu' }));

		await fireEvent.keyDown(document, { key: 'Escape' });

		expect(queryByTestId('chrome-menu')).toBeNull();
	});

	it('closes on a pointer outside it, and stays open for one inside it', async () => {
		const { getByRole, queryByTestId } = mount();
		await fireEvent.click(getByRole('button', { name: 'Menu' }));

		await fireEvent.pointerDown(getByRole('button', { name: 'Follow system' }));
		expect(queryByTestId('chrome-menu')).not.toBeNull();

		await fireEvent.pointerDown(document.body);
		expect(queryByTestId('chrome-menu')).toBeNull();
	});
});
