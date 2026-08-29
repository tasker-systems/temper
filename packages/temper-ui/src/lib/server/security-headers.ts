/**
 * The response headers every UI-rendered response carries.
 *
 * **Set here, not at the edge.** The hosting platform may well send some of these; it can also be
 * reconfigured, or replaced, without any change to this repository. A control that holds only while
 * something in front of the app keeps behaving is not a control this codebase can claim. The Rust
 * surfaces set the same baseline in `temper_services::transport::apply_base_layers`, for the same
 * reason and with the same values.
 *
 * **Content-Security-Policy is deliberately absent from this set.** It is the header that matters
 * most on a browser-facing surface and the one that cannot be added by copying a list: SvelteKit
 * emits inline scripts and styles, and the graph views set inline styles from d3 at runtime. Adding
 * it is its own piece of work, tracked separately — see the note in `svelte.config.js`.
 *
 * Applied only to responses this app renders. Proxied API/MCP/OAuth paths short-circuit before this
 * runs and carry the upstream's own headers, which the upstream sets for itself — one owner per
 * response rather than two writers racing over the same names.
 */
export const SECURITY_HEADERS: ReadonlyArray<readonly [string, string]> = [
	// Both this app and the API answer with content types they set deliberately. Sniffing can only
	// ever disagree with one of them.
	['x-content-type-options', 'nosniff'],
	// No page here is meant to be framed. Kept alongside the eventual `frame-ancestors`, which is
	// what actually enforces this in a modern browser — this is for the ones that never learned.
	['x-frame-options', 'DENY'],
	// The auth callback routes carry state in their URLs. A referrer leak from one of them is the
	// clearest path this app has to handing a URL to a third party.
	['referrer-policy', 'no-referrer'],
	// Two years — the usual floor for preload eligibility. `preload` itself is NOT sent: it is a
	// submission to a browser-vendor list that is painful to reverse, and so an operator's
	// decision rather than a default this repository makes for every install.
	['strict-transport-security', 'max-age=63072000; includeSubDomains'],
];

/**
 * Add the baseline to a response's headers, leaving any the response already set.
 *
 * `if not present` rather than overwrite, matching the Rust surfaces: it makes the baseline a floor
 * a route can raise or relax for its own content, rather than a value layered over one.
 */
export function applySecurityHeaders(headers: Headers): void {
	for (const [name, value] of SECURITY_HEADERS) {
		if (!headers.has(name)) headers.set(name, value);
	}
}
