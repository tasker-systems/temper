// sanitize.ts — the client-only DOMPurify pass for rendered markdown.
//
// markdown.ts stays DOM-FREE (servable both sides); this module owns the browser-only DOM
// dependency and the one-time hook registration, so MarkdownRenderer's dynamic import under
// `if (browser)` never double-adds the hook across remounts.
//
// Two attribute channels can carry author-controlled styling into the rendered document; both
// are closed here, in one pass:
//
// • style — FORBID_ATTR. The CSP deliberately admits style attributes
//   (`style-src-attr 'unsafe-inline'` in svelte.config.js — d3 and app.html set them at
//   runtime), so for authored content the sanitizer is the gate.
//
// • class — filtered to the prefixes the markdown pipeline legitimately emits. The CSP has no
//   lever for this channel: a class value is neither a style attribute nor a fetch. But this
//   build is Tailwind v4, so its positioning utilities ship in the production stylesheet and a
//   surviving class value still styles the rendered document. The allowlist is exhaustive over
//   the pipeline's own output: `hljs` / `hljs-*` (highlight.js theme hook and token spans),
//   `language-*` (marked's langPrefix, also what copy affordances key on), and `md-refusal`
//   (the static REFUSAL_HTML). Deliberately NOT `FORBID_ATTR: ['class']`, which would strip
//   those wholesale. The `.md-body` wrapper is added by the component outside the sanitized
//   string and needs no entry.
import DOMPurify from 'dompurify';

const keepClass = (value: string): boolean =>
	value === 'hljs' ||
	value.startsWith('hljs-') ||
	value.startsWith('language-') ||
	value === 'md-refusal';

DOMPurify.addHook('afterSanitizeAttributes', (node) => {
	if (!(node instanceof Element) || !node.hasAttribute('class')) return;
	const kept = (node.getAttribute('class') ?? '').split(/\s+/).filter(Boolean).filter(keepClass);
	if (kept.length) node.setAttribute('class', kept.join(' '));
	else node.removeAttribute('class');
});

export const SANITIZE_CONFIG = { FORBID_ATTR: ['style'] };

export function sanitizeMarkdownHtml(dirty: string): string {
	// `as unknown as string`: under the browser types sanitize's configured overload is typed
	// TrustedHTML; RETURN_TRUSTED_TYPE is unset, so the runtime value is a plain string.
	return DOMPurify.sanitize(dirty, SANITIZE_CONFIG) as unknown as string;
}
