// markdown.ts — the one Marked instance, with fenced-block syntax highlighting, and the one
// markdown→HTML preparation.
//
// An isolated `Marked` instance rather than `marked.use()` on the module singleton: a global
// mutation would silently reconfigure every other consumer of the package, and the instance
// makes the highlighting pipeline unit-testable without a component or a DOM.
//
// highlight.js over Shiki here because the pipeline is synchronous end to end — `parse` feeds
// the sanitizer pass in `MarkdownRenderer`, and an async highlighter would force that whole
// chain to restructure for prettier output. Language registration and the
// unknown-language-means-plaintext rule live in `highlight.ts`, shared with the direct-code
// JSON viewers (`ArtifactList` content, `ShapeList` schema).
//
// This module is DOM-FREE, deliberately, so it is servable in both environments. Sanitization
// is client-only: DOMPurify's server-side story requires a DOM emulation stack (jsdom) whose
// transitive tree does not load on every serverless runtime, and a security gate must not be
// runtime-sensitive. `MarkdownRenderer` gates the {@html} on the client-loaded sanitizer
// (`$lib/sanitize` owns the pass and its config) and never falls back to unsanitized output —
// which is why `prepareMarkdown` may return parse output that only the sanitizer may render.
import { Marked } from 'marked';
import { markedHighlight } from 'marked-highlight';
import { highlightCode } from '$lib/highlight';

const marked = new Marked(
	markedHighlight({
		// Both classes: `language-*` is what marked emits (and what copy-to-clipboard
		// affordances key on); `hljs` is the theme hook.
		langPrefix: 'hljs language-',
		highlight(code, lang) {
			return highlightCode(code, lang || undefined);
		},
	}),
);

/** Parse markdown to HTML, with fenced code blocks highlighted per the registered set. */
export function parseMarkdown(markdown: string): string {
	return marked.parse(markdown, { async: false }) as string;
}

/**
 * Cap on source markdown entering the parse. Bounds marked's superlinear worst cases (deep
 * nesting, emphasis runs) at the gate; real documents sit orders of magnitude below it.
 */
export const MAX_SOURCE_LENGTH = 262_144;

/**
 * What renders when source is past the bound or shaped so marked cannot parse it. Static, so
 * `{@html}` receives exactly this string and nothing user-shaped; it passes through the
 * sanitizer unchanged.
 */
export const REFUSAL_HTML = '<p class="md-refusal">This document could not be rendered.</p>';

/**
 * Markdown → HTML without the sanitizer, in one synchronous pass for both environments. Total:
 * source past the bound, or a shape marked cannot parse, renders {@link REFUSAL_HTML} instead
 * of throwing out of the caller's derive.
 *
 * The result is NOT safe for `{@html}` on its own — the client-side sanitizer owns that step.
 * This is still the sanctioned source of markdown HTML: parse and the bound are composed here,
 * never separately at a call site.
 */
export function prepareMarkdown(markdown: string): string {
	if (markdown.length > MAX_SOURCE_LENGTH) return REFUSAL_HTML;
	try {
		return parseMarkdown(markdown);
	} catch {
		return REFUSAL_HTML;
	}
}
