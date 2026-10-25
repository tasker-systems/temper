// sanitize.ts — the one sanitizer pass for rendered markdown, on every surface.
//
// `rendering-agrees`: what one surface strips or refuses, the other does not render. So there is
// one policy, and it lives here; why a given surface's CSP backs it up is that surface's comment
// to keep, next to its CSP, not this module's.
//
// markdown.ts stays DOM-FREE; this module owns the DOM dependency and the one-time hook
// registration on its own DOMPurify instance, so a renderer that dynamic-imports it on the
// client never double-adds the hook across remounts.
//
// Two attribute channels can carry author-controlled styling into the rendered document; both
// are closed here, in one pass:
//
// • style — FORBID_ATTR. Whatever a surface's CSP says about style attributes, the sanitizer is
//   the gate for authored content.
//
// • class — filtered to the prefixes the markdown pipeline legitimately emits. A CSP has no
//   lever for this channel: a class value is neither a style attribute nor a fetch. But the
//   surfaces are Tailwind v4, so their positioning utilities ship in the stylesheet and a
//   surviving class value still styles the rendered document. The allowlist is exhaustive over
//   the pipeline's own output: `hljs` / `hljs-*` (highlight.js theme hook and token spans),
//   `language-*` (marked's langPrefix, also what copy affordances key on), and `md-refusal`
//   (the static REFUSAL_HTML). Deliberately NOT `FORBID_ATTR: ['class']`, which would strip
//   those wholesale. A renderer's own wrapper (`.md-body`) is added outside the sanitized string
//   and needs no entry.
//
// Nothing in a rendered body fetches or submits: a body is rendered from what temper holds.
//
// • Images are withheld, as a declared refusal. After the pass, every image whose source is not
//   an embedded `data:image/` URI is replaced by a `md-withheld` span that says so, keeping its
//   alt text and naming the source on hover — never a silent broken glyph. `md-withheld` is
//   deliberately NOT in the class allowlist: it is added after sanitizing, so authored content
//   cannot forge a refusal.
//
// • Media is withheld the same way: a `<video>` or `<audio>` — with its sources, tracks and
//   poster — becomes a `md-withheld` span naming what it was, carrying the author's fallback text
//   as an image's note carries its alt, and naming its source on hover. A reader is told what the
//   body held; nothing disappears without a word.
//
// • What is left is removed, because it carries nothing a reader would miss: `srcset` (an
//   `<img>` or a `<picture>`'s `<source>` keeps its embedded `src`), the legacy `background`
//   attribute, and SVG `<image>` and `<feImage>` — decoration inside a drawing, with no place in
//   it for a sentence. A CSP refuses a cross-origin fetch but admits a
//   same-origin one, which on a surface with a session is a request made with the reader's
//   credentials — so the sanitizer is the gate here too.
//
// • Nothing submits: `<form>` and `<button>` are removed and their words kept, and every
//   `<input>` but the disabled checkbox a GFM task list renders is removed.
//
// • `<template>` is removed with its content, which a query of the document never reaches.
import DOMPurify from 'dompurify';
import { ABSENT_WORDS, type LinkTable, UNCHECKED_WORDS, vaultLinkId } from './links.js';
import { regionWords } from './region.js';

const keepClass = (value: string): boolean =>
	value === 'hljs' ||
	value.startsWith('hljs-') ||
	value.startsWith('language-') ||
	value === 'md-refusal';

// An instance of our own: the hook below is this policy's, and registering it on the shared
// singleton would filter the classes of every other DOMPurify call in a consumer's bundle.
const purify = DOMPurify();

purify.addHook('afterSanitizeAttributes', (node) => {
	if (!(node instanceof Element) || !node.hasAttribute('class')) return;
	const kept = (node.getAttribute('class') ?? '').split(/\s+/).filter(Boolean).filter(keepClass);
	if (kept.length) node.setAttribute('class', kept.join(' '));
	else node.removeAttribute('class');
});

export const SANITIZE_CONFIG = {
	FORBID_ATTR: ['style', 'srcset', 'background'],
	// `action` and `formaction` go with the form and button, and `<input>`'s with the pass below;
	// neither needs its own entry.
	FORBID_TAGS: ['image', 'feImage', 'form', 'button', 'template'],
};

/** Words for a withheld image; exported so the witnesses and the components agree. */
export const WITHHELD_IMAGE = 'remote image withheld';

/** Words for withheld media, by element; exported for the same reason. */
export const WITHHELD_MEDIA = { video: 'video withheld', audio: 'audio withheld' } as const;

const embedded = (src: string | null): boolean => /^data:image\//i.test(src?.trim() ?? '');

function withheld(words: string, src: string | null | undefined, doc: Document): HTMLSpanElement {
	const span = doc.createElement('span');
	span.className = 'md-withheld';
	span.textContent = words;
	if (src?.trim()) span.title = src.trim();
	return span;
}

function withheldImage(img: HTMLImageElement): HTMLSpanElement {
	const alt = img.getAttribute('alt')?.trim();
	const words = alt ? `${WITHHELD_IMAGE}: ${alt}` : WITHHELD_IMAGE;
	return withheld(words, img.getAttribute('src'), img.ownerDocument);
}

function withheldMedia(
	media: HTMLMediaElement,
	kind: keyof typeof WITHHELD_MEDIA,
): HTMLSpanElement {
	const src = media.getAttribute('src') ?? media.querySelector('source')?.getAttribute('src');
	// The author's fallback, as an image's note carries its alt: a `<source>` or `<track>` holds no
	// text, so what remains is the words written for a reader who cannot play it.
	const fallback = media.textContent?.replace(/\s+/g, ' ').trim();
	const words = fallback ? `${WITHHELD_MEDIA[kind]}: ${fallback}` : WITHHELD_MEDIA[kind];
	return withheld(words, src, media.ownerDocument);
}

/**
 * How a body's vault links render: the answers temper gave and the surface's own resource
 * address, or `'pending'` while the body is still arriving (a streamed reply), when links are
 * inert typed text and say nothing yet.
 */
export type BodyLinks = { table: LinkTable; hrefFor: (id: string) => string } | 'pending';

// Each state takes its marker from the region vocabulary, as `empty` and `failed` keep apart there:
// absent is temper saying there is nothing you can see; unchecked is nothing verified either way.
const ABSENT_MARKER = regionWords('empty', 'link').marker;
const UNCHECKED_MARKER = regionWords('failed', 'link').marker;

const XHTML = 'http://www.w3.org/1999/xhtml';
const XLINK = 'http://www.w3.org/1999/xlink';

function marker(glyph: string, doc: Document): HTMLSpanElement {
	const span = doc.createElement('span');
	span.className = 'md-ref-marker';
	span.setAttribute('aria-hidden', 'true');
	span.textContent = `${glyph}\u2009`;
	return span;
}

/**
 * Rewrite every vault link from the answers. After sanitizing, so the classes and words it sets
 * are the pass's own: `md-ref-*` is not in {@link keepClass}, so an author's copy is already gone,
 * and nothing here reads an authored attribute but the href. Every vault anchor is rebuilt — an
 * authored href is never left live:
 *
 * - **resolved** — the held title, as text, at the surface's address.
 * - **absent** — inert: the link's own content in a span, with one sentence for gone and not
 *   visible.
 * - **unchecked** (no answer, or no table) — still followable, since the destination does its own
 *   read and following grants nothing; marked, with the link's own content, at the surface's
 *   address.
 * - **pending** — the link's own content in place, inert and unmarked.
 *
 * **An anchor in foreign content (SVG) is always typed text**, whatever its answer, and by
 * `xlink:href` as well as `href`. Putting an HTML element inside a drawing makes the serialized
 * string parse back into a different DOM than the one DOMPurify approved — a `<span>` closes the
 * `<svg>` on reparse — and the sanitizer's guarantee holds only for the DOM it saw. Text cannot
 * break out, and leaves no authored href live.
 */
function rewriteVaultLinks(fragment: DocumentFragment, links: BodyLinks | undefined): void {
	const doc = fragment.ownerDocument;
	for (const a of fragment.querySelectorAll('a')) {
		const id = vaultLinkId(a.getAttribute('href') ?? a.getAttributeNS(XLINK, 'href') ?? '');
		if (!id) continue;
		if (a.namespaceURI !== XHTML) {
			a.replaceWith(doc.createTextNode(a.textContent ?? ''));
			continue;
		}
		// The link's own content, already sanitized above: an inline `code`, an embedded image, a
		// withheld note. Moved, not flattened to text, so nothing in it disappears without a word.
		const content = [...a.childNodes];
		if (links === 'pending') {
			a.replaceWith(...content);
			continue;
		}
		const answer = links?.table.get(id);
		if (answer?.state === 'absent') {
			const span = doc.createElement('span');
			span.className = 'md-ref-absent';
			span.title = ABSENT_WORDS;
			span.append(marker(ABSENT_MARKER, doc), ...content);
			a.replaceWith(span);
			continue;
		}
		const rebuilt = doc.createElement('a');
		rebuilt.setAttribute('href', links ? links.hrefFor(id) : `./${id}`);
		if (answer?.state === 'resolved') {
			rebuilt.textContent = answer.title;
		} else {
			rebuilt.className = 'md-ref-unchecked';
			rebuilt.title = UNCHECKED_WORDS;
			rebuilt.append(marker(UNCHECKED_MARKER, doc), ...content);
		}
		a.replaceWith(rebuilt);
	}
}

export function sanitizeMarkdownHtml(dirty: string, links?: BodyLinks): string {
	const fragment = purify.sanitize(dirty, { ...SANITIZE_CONFIG, RETURN_DOM_FRAGMENT: true });
	for (const img of fragment.querySelectorAll('img')) {
		if (!embedded(img.getAttribute('src'))) img.replaceWith(withheldImage(img));
	}
	for (const kind of ['video', 'audio'] as const) {
		for (const media of fragment.querySelectorAll(kind))
			media.replaceWith(withheldMedia(media, kind));
	}
	for (const input of fragment.querySelectorAll('input')) {
		if (input.getAttribute('type') !== 'checkbox') input.remove();
	}
	rewriteVaultLinks(fragment, links);
	// The holder comes from the fragment's own inert document, not the live one: adopting the
	// fragment into a live document would run its elements' steps before `{@html}` does.
	const holder = fragment.ownerDocument.createElement('div');
	holder.append(fragment);
	return holder.innerHTML;
}
