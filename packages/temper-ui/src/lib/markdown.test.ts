import { describe, expect, it } from 'vitest';
import { parseMarkdown } from './markdown';

/**
 * The highlighting pipeline's contract, independent of the component: fenced blocks are
 * highlighted ONLY for the registered languages; anything else — unknown, misspelled, or no
 * fence language — renders plaintext-escaped, never auto-detected; and inline code stays
 * un-highlighted. The component's DOMPurify pass runs after this, so the output here is the
 * sanitizer's INPUT — a `<script>` must arrive escaped, or the sanitizer has work to do that
 * this pipeline invented for it.
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
