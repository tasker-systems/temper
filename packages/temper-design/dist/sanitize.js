// sanitize.ts — the one sanitizer pass for rendered markdown, on every surface.
//
// `rendering-agrees`: what one surface strips or refuses, the other does not render. So there is
// one policy, and it lives here; why a given surface's CSP backs it up is that surface's comment
// to keep, next to its CSP, not this module's.
//
// markdown.ts stays DOM-FREE; this module owns the DOM dependency and the one-time hook
// registration, so a renderer that dynamic-imports it on the client never double-adds the hook
// across remounts.
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
// Remote images are withheld, as a declared refusal. A body is rendered from what temper holds,
// and nothing in it fetches: after the pass, every image whose source is not an embedded
// `data:image/` URI is replaced by a `md-withheld` span that says so, keeping its alt text and
// naming the source on hover — never a silent broken glyph. `md-withheld` is deliberately NOT in
// the class allowlist: it is added after sanitizing, so authored content cannot forge a refusal.
import DOMPurify from 'dompurify';
const keepClass = (value) => value === 'hljs' ||
    value.startsWith('hljs-') ||
    value.startsWith('language-') ||
    value === 'md-refusal';
DOMPurify.addHook('afterSanitizeAttributes', (node) => {
    if (!(node instanceof Element) || !node.hasAttribute('class'))
        return;
    const kept = (node.getAttribute('class') ?? '').split(/\s+/).filter(Boolean).filter(keepClass);
    if (kept.length)
        node.setAttribute('class', kept.join(' '));
    else
        node.removeAttribute('class');
});
export const SANITIZE_CONFIG = { FORBID_ATTR: ['style'] };
/** Words for a withheld image; exported so the witnesses and the components agree. */
export const WITHHELD_IMAGE = 'remote image withheld';
const embedded = (src) => /^data:image\//i.test(src?.trim() ?? '');
function withheld(img) {
    const span = img.ownerDocument.createElement('span');
    span.className = 'md-withheld';
    const alt = img.getAttribute('alt')?.trim();
    span.textContent = alt ? `${WITHHELD_IMAGE}: ${alt}` : WITHHELD_IMAGE;
    const src = img.getAttribute('src')?.trim();
    if (src)
        span.title = src;
    return span;
}
export function sanitizeMarkdownHtml(dirty) {
    const fragment = DOMPurify.sanitize(dirty, { ...SANITIZE_CONFIG, RETURN_DOM_FRAGMENT: true });
    for (const img of fragment.querySelectorAll('img')) {
        if (!embedded(img.getAttribute('src')))
            img.replaceWith(withheld(img));
    }
    const holder = document.createElement('div');
    holder.append(fragment);
    return holder.innerHTML;
}
