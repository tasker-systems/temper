import { render } from 'svelte/server';
import { describe, expect, it, vi } from 'vitest';
import { setPage } from '../../test/app-context';
import ThemeSwitch from './ThemeSwitch.svelte';

vi.mock('$app/stores', () => import('../../test/app-context'));

/**
 * The server renders the switch before the store starts. Its pressed action there is the one the
 * `(app)` layout read from the cookie (`themePreference`), so a reader who fixed a theme never
 * sees "Follow system" pressed for a frame. A unit test, because only that project renders Svelte
 * for the server.
 */
describe('the appearance control, rendered on the server', () => {
	it('presses the reader’s choice before the store starts', () => {
		setPage(
			'/vault/all',
			{},
			{ themePreference: { follow: 'fixed', name: 'quiet-instrument-paper' } },
		);
		const { body } = render(ThemeSwitch);
		expect(body).toMatch(/aria-pressed="true"[^>]*>\s*Quiet Instrument — Paper/);
		expect(body).toMatch(/aria-pressed="false"[^>]*>\s*Follow system/);
	});
});
