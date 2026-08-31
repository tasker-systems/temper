// markdown.ts — the one Marked instance, with fenced-block syntax highlighting.
//
// An isolated `Marked` instance rather than `marked.use()` on the module singleton: a global
// mutation would silently reconfigure every other consumer of the package, and the instance
// makes the highlighting pipeline unit-testable without a component or a DOM.
//
// highlight.js over Shiki here because the pipeline is synchronous end to end — `parse` feeds
// a client-side DOMPurify pass in `MarkdownRenderer`, and an async highlighter would force
// that whole chain to restructure for prettier output. Languages are REGISTERED, not bundled:
// the set below is what vault bodies actually carry (agents write json/js/ts/bash/rust/yaml/
// sql and diffs), and each registration is a small grammar rather than the ~1 MB common bundle.
// A fence whose language is not registered is highlighted as plaintext — never auto-detected,
// because a misfire colors prose as code, which reads as wrong more often than it reads as
// helpful.

import hljs from 'highlight.js/lib/core';
import bash from 'highlight.js/lib/languages/bash';
import diff from 'highlight.js/lib/languages/diff';
import javascript from 'highlight.js/lib/languages/javascript';
import json from 'highlight.js/lib/languages/json';
import plaintext from 'highlight.js/lib/languages/plaintext';
import rust from 'highlight.js/lib/languages/rust';
import sql from 'highlight.js/lib/languages/sql';
import typescript from 'highlight.js/lib/languages/typescript';
import xml from 'highlight.js/lib/languages/xml';
import yaml from 'highlight.js/lib/languages/yaml';
import { Marked } from 'marked';
import { markedHighlight } from 'marked-highlight';

hljs.registerLanguage('bash', bash);
hljs.registerLanguage('diff', diff);
hljs.registerLanguage('javascript', javascript);
hljs.registerLanguage('json', json);
hljs.registerLanguage('plaintext', plaintext);
hljs.registerLanguage('rust', rust);
hljs.registerLanguage('sql', sql);
hljs.registerLanguage('typescript', typescript);
hljs.registerLanguage('xml', xml);
hljs.registerLanguage('yaml', yaml);
// Aliases the fences actually use — `js`, `ts`, `sh` and friends map onto registered grammars.
hljs.registerAliases(['js', 'jsx'], { languageName: 'javascript' });
hljs.registerAliases(['ts', 'tsx'], { languageName: 'typescript' });
hljs.registerAliases(['sh', 'shell', 'zsh'], { languageName: 'bash' });
hljs.registerAliases(['yml'], { languageName: 'yaml' });
hljs.registerAliases(['html', 'svelte'], { languageName: 'xml' });

const marked = new Marked(
	markedHighlight({
		// Both classes: `language-*` is what marked emits (and what copy-to-clipboard
		// affordances key on); `hljs` is the theme hook.
		langPrefix: 'hljs language-',
		highlight(code, lang) {
			// A language we registered highlights; anything else — unknown, misspelled, or no
			// fence language at all — is plaintext, escaped and uncolored. Auto-detection is
			// the rejected option: it colors by confidence, and a wrong guess in a document
			// reader is worse than no color at all.
			if (lang && hljs.getLanguage(lang)) {
				return hljs.highlight(code, { language: lang }).value;
			}
			return hljs.highlight(code, { language: 'plaintext' }).value;
		},
	}),
);

/** Parse markdown to HTML, with fenced code blocks highlighted per the registered set. */
export function parseMarkdown(markdown: string): string {
	return marked.parse(markdown, { async: false }) as string;
}
