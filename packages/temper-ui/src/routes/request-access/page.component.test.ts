import { render } from '@testing-library/svelte';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { resetAppContext } from '../../test/app-context';
import Page from './+page.svelte';

vi.mock('$app/navigation', () => import('../../test/app-context'));
vi.mock('$app/forms', () => ({ enhance: () => ({ destroy: () => {} }) }));

beforeEach(resetAppContext);

const mount = (data: Record<string, unknown>) =>
	render(Page, {
		data: {
			user: { sub: 'user-1' },
			ownRequest: null,
			...data,
		},
		form: null,
	} as never);

const SETTINGS = {
	terms_version: null,
	terms_resource_uri: null,
	instance_name: null,
};

describe('the request-access header — the instance name is shown only when a read carried it', () => {
	it('renders the instance name the settings read answered', () => {
		const { container } = mount({
			settings: { ...SETTINGS, instance_name: 'Steward Archive' },
		});

		expect(container.textContent).toContain('Steward Archive · invitation only');
	});

	it('renders no instance name when the settings read failed — never a fabricated domain', () => {
		const { container } = mount({ settings: null });

		expect(container.textContent).toContain('invitation only');
		expect(container.textContent).not.toContain('temperkb.io');
	});

	it('renders no instance name when the settings carry none — never a fabricated domain', () => {
		const { container } = mount({ settings: SETTINGS });

		expect(container.textContent).toContain('invitation only');
		expect(container.textContent).not.toContain('temperkb.io');
	});
});
