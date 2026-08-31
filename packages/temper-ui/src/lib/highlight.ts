// highlight.ts — the one highlight.js registration, and the direct-code highlight helper.
//
// `markdown.ts` (fenced blocks in document bodies) and the JSON viewers (`ArtifactList`
// content, `ShapeList` schema) share this: one registration list, one set of span classes,
// one theme wherever `.hljs` appears.
//
// **Why `{@html highlightCode(...)}` is safe by construction:** hljs escapes the input text
// and emits only its own `<span class="hljs-*">` wrappers — no input byte can become markup.
// This is the documented property that makes it the standard choice for code viewers that
// render arbitrary stored content.
//
// Unknown languages are PLAINTEXT, never auto-detected: a wrong guess colors prose as code,
// which reads as wrong more often than it reads as helpful. `plaintext` is registered
// explicitly — `hljs/lib/core` does not ship it pre-registered (a full-bundle behavior the
// core entry silently lacks).
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

/** Highlight `code` as `lang` (or plaintext when unregistered) into safe hljs HTML. */
export function highlightCode(code: string, lang?: string): string {
	if (lang && hljs.getLanguage(lang)) {
		return hljs.highlight(code, { language: lang }).value;
	}
	return hljs.highlight(code, { language: 'plaintext' }).value;
}
