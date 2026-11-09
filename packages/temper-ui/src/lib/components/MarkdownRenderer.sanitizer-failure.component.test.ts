import { render, waitFor } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import MarkdownRenderer from './MarkdownRenderer.svelte';

/**
 * The branch where the sanitizer's chunk never arrives (a deploy rotated it away, a network drop).
 * Its own file because the failing import is a file-scoped module mock; the rest of the
 * renderer's wiring is in `MarkdownRenderer.component.test.ts`.
 *
 * The property: the body is not left blank, and what renders instead is the source as text —
 * never the unsanitized HTML.
 */
vi.mock('@tasker-systems/temper-design/sanitize', () => {
	throw new Error('chunk load failed');
});

const DIRTY = '# Temper\n\n<script>alert(1)</script>\n\nA [link](https://example.com).';

describe('MarkdownRenderer when the sanitizer fails to load', () => {
	afterEach(() => {
		vi.restoreAllMocks();
	});

	it('renders the source as escaped text under a notice, never as HTML', async () => {
		const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
		const { container } = render(MarkdownRenderer, { props: { markdown: DIRTY } });

		await waitFor(() => expect(container.querySelector('.md-load-failed')).not.toBeNull());
		expect(container.querySelector('.md-body')).toBeNull();
		expect(container.querySelector('script')).toBeNull();
		expect(container.querySelector('h1')).toBeNull();
		expect(container.querySelector('a')).toBeNull();
		expect(container.querySelector('pre')?.textContent).toBe(DIRTY);
		expect(container.querySelector('[role="status"]')).not.toBeNull();
		expect(errorSpy).toHaveBeenCalledTimes(1);
	});
});
