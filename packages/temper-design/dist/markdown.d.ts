/** Parse markdown to HTML, with fenced code blocks highlighted per the registered set. */
export declare function parseMarkdown(markdown: string): string;
/**
 * Cap on source markdown entering the parse. Bounds marked's superlinear worst cases (deep
 * nesting, emphasis runs) at the gate; real documents sit orders of magnitude below it.
 */
export declare const MAX_SOURCE_LENGTH = 262144;
/**
 * What renders when source is past the bound or shaped so marked cannot parse it. Static, so
 * `{@html}` receives exactly this string and nothing user-shaped; it passes through the
 * sanitizer unchanged.
 */
export declare const REFUSAL_HTML = "<p class=\"md-refusal\">This document could not be rendered.</p>";
/**
 * Markdown → HTML without the sanitizer, in one synchronous pass. Total: source past the bound,
 * or a shape marked cannot parse, renders {@link REFUSAL_HTML} instead of throwing out of the
 * caller's derive.
 *
 * The result is NOT safe for `{@html}` on its own — the sanitizer owns that step. This is still
 * the sanctioned source of markdown HTML: parse and the bound are composed here, never
 * separately at a call site.
 */
export declare function prepareMarkdown(markdown: string): string;
