import { describe, expect, it, vi } from 'vitest';
import { highlightCode } from './highlight';
import { MAX_SOURCE_LENGTH, parseMarkdown, prepareMarkdown, REFUSAL_HTML } from './markdown';

/**
 * The highlighting pipeline's contract, independent of the component: fenced blocks are
 * highlighted ONLY for the registered languages; anything else — unknown, misspelled, or no
 * fence language — renders plaintext-escaped, never auto-detected; and inline code stays
 * un-highlighted. The client-side sanitizer composes with this output in `MarkdownRenderer`, so
 * the output here is the sanitizer's INPUT — a `<script>` must arrive escaped, or the sanitizer
 * has work to do that this pipeline invented for it.
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
 * The bound-and-refuse contract: `prepareMarkdown` is total. Source past the length bound and
 * shapes marked cannot parse render as the static refusal — never a throw out of the derive,
 * which on the server would fail the page for every reader. Its output is not safe for
 * `{@html}` by itself — the client-side sanitizer owns that step — so the payload battery lives
 * in the component suite, on the layer where sanitization actually runs.
 */
describe('prepareMarkdown', () => {
	it('passes an empty body through as empty', () => {
		expect(prepareMarkdown('')).toBe('');
	});

	it('refuses to parse source past the length bound', () => {
		expect(prepareMarkdown('a'.repeat(MAX_SOURCE_LENGTH + 1))).toBe(REFUSAL_HTML);
	});

	// The depth at which marked's recursive tokenizer overflows is a property of the runtime's
	// stack, not of the input — one machine throws `RangeError` on 2000 nested blockquotes,
	// another parses them fine. So the parser-throw arm is witnessed with the throw injected
	// through a scoped `marked` mock: the contract "a parse throw never escapes prepareMarkdown"
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
			expect(isolated.prepareMarkdown('> deep')).toBe(REFUSAL_HTML);
		} finally {
			vi.doUnmock('marked');
			vi.resetModules();
		}
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
