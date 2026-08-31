// markdown.ts — the one Marked instance, with fenced-block syntax highlighting.
//
// An isolated `Marked` instance rather than `marked.use()` on the module singleton: a global
// mutation would silently reconfigure every other consumer of the package, and the instance
// makes the highlighting pipeline unit-testable without a component or a DOM.
//
// highlight.js over Shiki here because the pipeline is synchronous end to end — `parse` feeds
// a client-side DOMPurify pass in `MarkdownRenderer`, and an async highlighter would force
// that whole chain to restructure for prettier output. Language registration and the
// unknown-language-means-plaintext rule live in `highlight.ts`, shared with the direct-code
// JSON viewers (`ArtifactList` content, `ShapeList` schema).
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
