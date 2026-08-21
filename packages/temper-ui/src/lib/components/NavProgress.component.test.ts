import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { resetAppContext, setNavigating } from '../../test/app-context';
import NavProgress from './NavProgress.svelte';

vi.mock('$app/stores', () => import('../../test/app-context'));

describe('navigation is acknowledged', () => {
	beforeEach(() => resetAppContext());

	it('C5: shows nothing while idle', () => {
		const { container } = render(NavProgress);
		expect(container.querySelector('[data-testid="nav-progress"]')).toBeNull();
	});

	it('C5: acknowledges a navigation the moment it starts', () => {
		setNavigating({ to: { url: new URL('http://localhost/vault/all') } });
		const { container } = render(NavProgress);
		const el = container.querySelector('[data-testid="nav-progress"]');
		expect(el).not.toBeNull();
		// Carries words, not only a visual bar — the accessibility tree is the point.
		expect(el?.textContent?.trim()).not.toBe('');
	});
});
