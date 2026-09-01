// markdown.ts — the one Marked instance, with fenced-block syntax highlighting, and the one
// markdown→safe-HTML composition.
//
// An isolated `Marked` instance rather than `marked.use()` on the module singleton: a global
// mutation would silently reconfigure every other consumer of the package, and the instance
// makes the highlighting pipeline unit-testable without a component or a DOM.
//
// highlight.js over Shiki here because the pipeline is synchronous end to end — `parse` feeds
// the DOMPurify pass in `renderMarkdown`, and an async highlighter would force that whole chain
// to restructure for prettier output. Language registration and the
// unknown-language-means-plaintext rule live in `highlight.ts`, shared with the direct-code
// JSON viewers (`ArtifactList` content, `ShapeList` schema).
//
// DOMPurify itself is browser-only; the `isomorphic-dompurify` wrapper is what lets the same
// synchronous call run in both environments (under the node export condition it supplies a
// jsdom window; in the browser it is DOMPurify's own entry). Keep the import on the wrapper —
// importing `dompurify` directly couples the pass to the browser again and takes the server
// render out of `renderMarkdown`'s guarantee.
import DOMPurify from 'isomorphic-dompurify';
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
 * Markdown → sanitizer-approved HTML, in one synchronous pass, for both environments.
 *
 * This is the sanctioned path from markdown to `{@html}`: parse and sanitize are composed here,
 * never separately at a call site, so no consumer can ship the parse output raw.
 */
export function renderMarkdown(markdown: string): string {
	return DOMPurify.sanitize(parseMarkdown(markdown));
}
