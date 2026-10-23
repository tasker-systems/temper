import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import ResourceLink from './ResourceLink.svelte';

const ID = '01a169f2-8c90-7308-995f-6ff1358a64d3';

/** `references-show-held-truth` — a reference links only once the read answered for it. */
describe('ResourceLink', () => {
	it('links a reference the read returned a title for, showing that title', () => {
		const { container } = render(ResourceLink, { id: ID, title: 'The held title' });
		const a = container.querySelector('a');
		expect(a?.getAttribute('href')).toBe(`/vault/r/${ID}`);
		expect(a?.textContent).toBe('The held title');
	});

	it.each([null, undefined, ''])(
		'renders a reference with no held title (%s) inert and unresolved',
		(title) => {
			const { container, getByTestId } = render(ResourceLink, { id: ID, title });
			expect(container.querySelector('a')).toBeNull();
			expect(getByTestId('unresolved-ref').textContent).toContain('Unresolved reference');
		},
	);
});
