import { describe, expect, it, vi } from 'vitest';
import { highlightCode } from './highlight';
import { MAX_SOURCE_LENGTH, parseMarkdown, REFUSAL_HTML, renderMarkdown } from './markdown';

/**
 * The highlighting pipeline's contract, independent of the component: fenced blocks are
 * highlighted ONLY for the registered languages; anything else — unknown, misspelled, or no
 * fence language — renders plaintext-escaped, never auto-detected; and inline code stays
 * un-highlighted. The sanitize pass composes with this output inside `renderMarkdown`, so the
 * output here is the sanitizer's INPUT — a `<script>` must arrive escaped, or the sanitizer has
 * work to do that this pipeline invented for it.
 */
describe('parseMarkdown', () => {
	it('highlights a registered language with hljs spans', () => {
		const html = parseMarkdown('```json\n{"name": "atlas", "p50_ms": 412}\n```');
		expect(html).toContain('class="hljs language-json"');
		expect(html).toContain('hljs-attr');
		expect(html).toContain('hljs-number');
		expect(html).toContain('hljs-string');
	});

	it('resolves language aliases the fences actually use', () => {
		const html = parseMarkdown('```ts\nconst x: number = 1;\n```');
		expect(html).toContain('class="hljs language-ts"');
		expect(html).toContain('hljs-keyword');
	});

	it('renders an unknown language as escaped plaintext, colored nothing', () => {
		const html = parseMarkdown('```klingon\nconst <b>x</b> = 1;\n```');
		expect(html).toContain('language-klingon');
		expect(html).not.toContain('hljs-attr');
		expect(html).not.toContain('hljs-keyword');
		// Escaped, not raw — the angle brackets arrive as text.
		expect(html).toContain('&lt;b&gt;');
	});

	it('renders a fence with no language at all as plaintext', () => {
		const html = parseMarkdown('```\nplain text\n```');
		expect(html).toContain('<code');
		expect(html).not.toContain('hljs-attr');
	});

	it('leaves inline code alone', () => {
		const html = parseMarkdown('a `{"x":1}` inline');
		expect(html).not.toContain('hljs-attr');
		expect(html).toContain('<code>{&quot;x&quot;:1}</code>');
	});

	it('hands the sanitizer escaped scripts, never live ones', () => {
		const html = parseMarkdown('```json\n{"a": "<script>alert(1)</script>"}\n```');
		expect(html).not.toContain('<script>alert');
		expect(html).toContain('&lt;script&gt;');
	});
});

/**
 * The composition the renderer ships: parse, then sanitize, one synchronous pass. This suite
 * runs in plain node — the same environment the server render sanitizes in — so the node run
 * itself is the witness that the pass is not browser-only. Both faces asserted per case: the
 * payload gone AND the benign structure around it retained, since a probe that only checks
 * absence passes vacuously against an empty string.
 */
describe('renderMarkdown', () => {
	it('strips a script payload and keeps the readable structure around it', () => {
		const html = renderMarkdown(
			'# Temper\n\n<script>alert(1)</script>\n\nA [link](https://example.com).',
		);
		expect(html).toContain('<h1>Temper</h1>');
		expect(html).toContain('href="https://example.com"');
		expect(html).not.toContain('<script');
		expect(html).not.toContain('alert');
	});

	it('keeps an element but drops its event-handler attribute', () => {
		const html = renderMarkdown('<img src="https://example.com/a.png" onerror="alert(1)">');
		expect(html).toContain('src="https://example.com/a.png"');
		expect(html).not.toContain('onerror');
	});

	it('renders a javascript: link as a link with no javascript: destination', () => {
		const html = renderMarkdown('[click](javascript:alert(1))');
		expect(html).toContain('<a');
		expect(html).not.toContain('javascript:');
	});

	it('passes an empty body through as empty', () => {
		expect(renderMarkdown('')).toBe('');
	});

	// The remaining cases are the payload classes a security review flagged as unrepresented:
	// namespace-confusion mXSS (the class behind the historical DOMPurify CVEs), obfuscated
	// URI schemes, DOM-clobbering names, and raw templates/iframes/base tags.
	it('defuses namespace-confusion mXSS across math/style boundaries', () => {
		const html = renderMarkdown(
			'<math><mtext><form><mglyph><style></math><img src onerror=alert(1)>',
		);
		expect(html).not.toContain('onerror');
		expect(html).not.toContain('<mglyph');
	});

	it('drops hrefs whose scheme hides behind entities or data payloads', () => {
		const entity = renderMarkdown('<a href="jav&#x09;ascript:alert(1)">x</a>');
		expect(entity).not.toContain('javascript:');
		expect(entity).not.toContain('alert');

		const dataUri = renderMarkdown('[x](data:text/html;base64,PHNjcmlwdD4=)');
		expect(dataUri).not.toContain('data:text/html');
	});

	it('strips DOM-clobbering name attributes from form controls', () => {
		const html = renderMarkdown('<form><input name="attributes"><input name="tagName"></form>');
		expect(html).not.toContain('name="attributes"');
		expect(html).not.toContain('name="tagName"');
	});

	it('strips template contents and removes iframe and base wholesale', () => {
		const html = renderMarkdown(
			'<template><script>alert(1)</script></template><iframe srcdoc="<script>alert(1)</script>"></iframe><base href="https://evil.example/">',
		);
		// DOMPurify keeps an inert empty <template> shell but strips everything inside it;
		// iframe and base are dropped entirely.
		expect(html).not.toContain('<script');
		expect(html).not.toContain('alert');
		expect(html).not.toContain('<iframe');
		expect(html).not.toContain('<base');
		expect(html).not.toContain('evil.example');
	});
});

/**
 * The bound-and-refuse contract: `renderMarkdown` is total. Source past the length bound and
 * shapes marked cannot parse render as the static refusal — never a throw out of the derive,
 * which on the server would fail the page for every reader. Benign content keeps rendering:
 * each witness keeps a retention face so an over-eager refusal cannot pass.
 */
describe('renderMarkdown guards', () => {
	it('refuses to parse source past the length bound', () => {
		expect(renderMarkdown('a'.repeat(MAX_SOURCE_LENGTH + 1))).toBe(REFUSAL_HTML);
	});

	// The depth at which marked's recursive tokenizer overflows is a property of the runtime's
	// stack, not of the input — one machine throws `RangeError` on 2000 nested blockquotes,
	// another parses them fine. So the parser-throw arm is witnessed with the throw injected
	// through a scoped `marked` mock: the contract "a parse throw never escapes renderMarkdown"
	// is pinned on every platform, not on whichever stack ran the suite.
	it('returns the refusal when the parser throws', async () => {
		vi.resetModules();
		vi.doMock('marked', async (importOriginal) => {
			const actual = await importOriginal<typeof import('marked')>();
			class ThrowingMarked {
				parse(): string {
					throw new RangeError('Maximum call stack size exceeded');
				}
			}
			return { ...actual, Marked: ThrowingMarked as unknown as typeof actual.Marked };
		});
		try {
			const isolated = await import('./markdown');
			expect(isolated.renderMarkdown('> deep')).toBe(REFUSAL_HTML);
		} finally {
			vi.doUnmock('marked');
			vi.resetModules();
		}
	});

	it('strips style attributes and keeps the elements that carried them', () => {
		const html = renderMarkdown('<p style="color:red;position:fixed" class="lead">stay</p>');
		expect(html).toContain('class="lead"');
		expect(html).toContain('stay');
		expect(html).not.toContain('style=');
	});
});

describe('highlightCode', () => {
	it('escapes the input and emits only hljs spans — the {@html}-safety property', () => {
		const html = highlightCode('{"a": "<b>text</b>"}', 'json');
		expect(html).toContain('&lt;b&gt;');
		expect(html).not.toContain('<b>');
		expect(html.startsWith('<span class="hljs-punctuation">')).toBe(true);
	});

	it('falls back to plaintext for an unregistered language', () => {
		expect(highlightCode('plain', 'klingon')).not.toContain('hljs-attr');
	});
});
