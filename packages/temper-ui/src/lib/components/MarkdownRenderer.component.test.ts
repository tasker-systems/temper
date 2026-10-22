import { MAX_SOURCE_LENGTH } from '@tasker-systems/temper-design/markdown';
import { render, waitFor } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import MarkdownRenderer from './MarkdownRenderer.svelte';

/**
 * The component layer — the wiring, not the sanitizer's decision table: the payload battery runs
 * against the one sanitizer every surface shares, in `@tasker-systems/temper-design`
 * (`src/sanitize.test.ts`). In jsdom `browser` is true, so these mount the real client path.
 *
 * The gate is the property only this layer can see: while the sanitizer is unavailable the body
 * renders NOTHING — at first paint, and on the server render — and after it settles the output
 * is sanitized.
 */
const DIRTY = '# Temper\n\n<script>alert(1)</script>\n\nA [link](https://example.com).';

describe('MarkdownRenderer', () => {
	it('renders nothing at first paint — the gate holds before the sanitizer exists', () => {
		const { container } = render(MarkdownRenderer, { props: { markdown: DIRTY } });
		// Synchronous, before the dynamic import's promise can have resolved.
		expect(container.querySelector('.md-body')).toBeNull();
		expect(container.querySelector('script')).toBeNull();
		expect(container.textContent).not.toContain('Temper');
	});

	it('renders sanitized content once the sanitizer settles', async () => {
		const { container } = render(MarkdownRenderer, { props: { markdown: DIRTY } });
		await waitFor(() => expect(container.querySelector('.md-body')).not.toBeNull());
		expect(container.querySelector('h1')?.textContent).toBe('Temper');
		expect(container.querySelector('a')?.getAttribute('href')).toBe('https://example.com');
		expect(container.querySelector('script')).toBeNull();
		expect(container.textContent).not.toContain('alert');
	});

	// Driven through the length bound rather than a parse-throwing shape: the depth at which
	// marked overflows is a runtime-stack property (see the design package's markdown.test.ts), but the bound is
	// deterministic everywhere.
	it('renders the refusal for a body it cannot render, after the gate', async () => {
		const { container } = render(MarkdownRenderer, {
			props: { markdown: 'a'.repeat(MAX_SOURCE_LENGTH + 1) },
		});
		await waitFor(() => expect(container.querySelector('.md-refusal')).not.toBeNull());
	});

	it('renders the fallback for an empty body, not an empty region', () => {
		const { container } = render(MarkdownRenderer, { props: { markdown: '' } });
		expect(container.querySelector('.md-body')).toBeNull();
		expect(container.textContent).toContain('No content available.');
	});
});
