import { render } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { MAX_SOURCE_LENGTH } from '../markdown';
import MarkdownRenderer from './MarkdownRenderer.svelte';

/**
 * The component layer — the wiring, not the sanitizer's decision table (that is
 * `markdown.test.ts`'s `renderMarkdown` suite, in node). What only this layer can see: the
 * markdown reaches the DOM already sanitized at first synchronous paint, and an empty body
 * renders the named fallback rather than a silent region.
 *
 * The payload fixture carries benign structure beside it on purpose — the assertions check
 * both faces, so a pass that stripped everything would not go green.
 */
const DIRTY = '# Temper\n\n<script>alert(1)</script>\n\nA [link](https://example.com).';

describe('MarkdownRenderer', () => {
	it('renders sanitized html at first paint — no raw window before any async pass', () => {
		const { container } = render(MarkdownRenderer, { props: { markdown: DIRTY } });
		expect(container.querySelector('script')).toBeNull();
		expect(container.querySelector('h1')?.textContent).toBe('Temper');
		expect(container.querySelector('a')?.getAttribute('href')).toBe('https://example.com');
	});

	it('renders the fallback for an empty body, not an empty region', () => {
		const { container } = render(MarkdownRenderer, { props: { markdown: '' } });
		expect(container.querySelector('.md-body')).toBeNull();
		expect(container.textContent).toContain('No content available.');
	});

	// Driven through the length bound rather than a parse-throwing shape: the depth at which
	// marked overflows is a runtime-stack property (see markdown.test.ts), but the bound is
	// deterministic everywhere.
	it('renders the refusal for a body it cannot render, instead of failing the render', () => {
		const { container } = render(MarkdownRenderer, {
			props: { markdown: 'a'.repeat(MAX_SOURCE_LENGTH + 1) },
		});
		expect(container.querySelector('.md-refusal')).not.toBeNull();
	});
});
