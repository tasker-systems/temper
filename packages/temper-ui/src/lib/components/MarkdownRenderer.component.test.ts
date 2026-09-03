import { render, waitFor } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import { MAX_SOURCE_LENGTH } from '../markdown';
import MarkdownRenderer from './MarkdownRenderer.svelte';

/**
 * The component layer — the wiring, not the sanitizer's decision table in the abstract: the
 * sanitizer runs HERE (client-side, dynamic import of `$lib/sanitize`), so the payload battery
 * lives on this layer too. In jsdom `browser` is true, so these mount the real client path.
 *
 * The gate is the property only this layer can see: while the sanitizer is unavailable the body
 * renders NOTHING — at first paint, and on the server render — and after it settles the output
 * is sanitized. Every payload case keeps a retention face, so a pass that stripped everything
 * would not go green.
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

	it('keeps style attributes and non-allowlisted classes out of the rendered document', async () => {
		const { container } = render(MarkdownRenderer, {
			props: { markdown: '<p style="color:red;position:fixed" class="lead">stay</p>' },
		});
		await waitFor(() => expect(container.querySelector('p')).not.toBeNull());
		expect(container.textContent).toContain('stay');
		expect(container.innerHTML).not.toContain('style=');
		// `lead` is not in the sanitize allowlist — its survival would mean the class
		// attribute channel is not fully closed.
		expect(container.innerHTML).not.toContain('lead');
	});

	it('keeps positioning utilities out of the rendered document — content survives, styling does not', async () => {
		const { container } = render(MarkdownRenderer, {
			props: { markdown: '<div class="fixed inset-0 z-50 bg-black/80">overlay</div>' },
		});
		await waitFor(() => expect(container.textContent).toContain('overlay'));
		for (const utility of ['fixed', 'inset-0', 'z-50', 'bg-black']) {
			expect(container.innerHTML).not.toContain(utility);
		}
	});

	it('preserves the classes the markdown pipeline legitimately emits', async () => {
		const { container } = render(MarkdownRenderer, {
			props: { markdown: '```json\n{"a":1}\n```' },
		});
		await waitFor(() => expect(container.querySelector('code')).not.toBeNull());
		const html = container.innerHTML;
		expect(html).toContain('class="hljs language-json"');
		expect(html).toContain('hljs-');
	});

	it('keeps an element but drops its event-handler attribute', async () => {
		const { container } = render(MarkdownRenderer, {
			props: { markdown: '<img src="https://example.com/a.png" onerror="alert(1)">' },
		});
		await waitFor(() =>
			expect(container.querySelector('img')?.getAttribute('src')).toBe('https://example.com/a.png'),
		);
		expect(container.innerHTML).not.toContain('onerror');
	});

	it('defuses namespace-confusion mXSS across math/style boundaries', async () => {
		const { container } = render(MarkdownRenderer, {
			props: { markdown: '<math><mtext><form><mglyph><style></math><img src onerror=alert(1)>' },
		});
		await waitFor(() => expect(container.querySelector('img')).toBeNull());
		expect(container.innerHTML).not.toContain('onerror');
		expect(container.innerHTML).not.toContain('<mglyph');
	});

	it('drops hrefs whose scheme hides behind entities or data payloads', async () => {
		const { container } = render(MarkdownRenderer, {
			props: {
				markdown:
					'<a href="jav&#x09;ascript:alert(1)">x</a>\n\n[x](data:text/html;base64,PHNjcmlwdD4=)',
			},
		});
		await waitFor(() => expect(container.querySelector('a')).not.toBeNull());
		expect(container.innerHTML).not.toContain('javascript:');
		expect(container.innerHTML).not.toContain('alert');
		expect(container.innerHTML).not.toContain('data:text/html');
	});

	it('strips DOM-clobbering name attributes from form controls', async () => {
		const { container } = render(MarkdownRenderer, {
			props: { markdown: '<form><input name="attributes"><input name="tagName"></form>' },
		});
		await waitFor(() => expect(container.querySelector('form')).not.toBeNull());
		expect(container.innerHTML).not.toContain('name="attributes"');
		expect(container.innerHTML).not.toContain('name="tagName"');
	});

	it('strips template contents and removes iframe and base wholesale', async () => {
		const { container } = render(MarkdownRenderer, {
			props: {
				markdown:
					'<template><script>alert(1)</script></template><iframe srcdoc="<script>alert(1)</script>"></iframe><base href="https://evil.example/">',
			},
		});
		await waitFor(() => expect(container.querySelector('.md-body')).not.toBeNull());
		// DOMPurify keeps an inert empty <template> shell but strips everything inside it;
		// iframe and base are dropped entirely.
		expect(container.innerHTML).not.toContain('<script');
		expect(container.innerHTML).not.toContain('alert');
		expect(container.innerHTML).not.toContain('<iframe');
		expect(container.innerHTML).not.toContain('<base');
		expect(container.innerHTML).not.toContain('evil.example');
	});

	// Driven through the length bound rather than a parse-throwing shape: the depth at which
	// marked overflows is a runtime-stack property (see markdown.test.ts), but the bound is
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
