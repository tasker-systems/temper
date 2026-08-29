import { describe, expect, it } from 'vitest';
import { applySecurityHeaders, SECURITY_HEADERS } from './security-headers';

describe('applySecurityHeaders', () => {
	it('sets the whole baseline on a response that carries none of it', () => {
		const headers = new Headers();

		applySecurityHeaders(headers);

		for (const [name, value] of SECURITY_HEADERS) {
			expect(headers.get(name), `${name} must be set in-app`).toBe(value);
		}
	});

	/**
	 * The baseline is a floor, not a value layered over one.
	 *
	 * This is the half that makes the design work rather than merely look tidy: the same rule on the
	 * Rust surfaces is what lets the Slack callback page carry its own content-security policy. A
	 * baseline that overwrote would silently undo any per-response decision, and every "is the header
	 * present" assertion would still pass.
	 */
	it('leaves a header the response already set', () => {
		const headers = new Headers({ 'referrer-policy': 'strict-origin-when-cross-origin' });

		applySecurityHeaders(headers);

		expect(headers.get('referrer-policy')).toBe('strict-origin-when-cross-origin');
		expect(headers.get('x-content-type-options')).toBe('nosniff');
	});

	/**
	 * `preload` is deliberately not sent. It is a submission to a browser-vendor list that is painful
	 * to reverse, so it is an operator's decision rather than a default this repository makes for
	 * every install — and the reasoning is worth pinning, because adding the token is a one-word edit
	 * that looks like strengthening and is very hard to undo.
	 */
	it('does not enrol the domain in HSTS preload', () => {
		const headers = new Headers();

		applySecurityHeaders(headers);

		expect(headers.get('strict-transport-security')).not.toContain('preload');
	});

	/**
	 * The content-security policy belongs to SvelteKit (`kit.csp` in svelte.config.js), which emits
	 * it inside `resolve` with the per-request nonce its inline hydration script needs. Were it also
	 * in this array, the hook would write a nonce-less policy over Kit's nonce-bearing one — and
	 * every "is the header set" assertion would still pass while the page broke.
	 */
	it('does not carry the content-security policy — that is SvelteKit\u2019s to emit', () => {
		const names = SECURITY_HEADERS.map(([name]) => name);

		expect(names).not.toContain('content-security-policy');
	});
});
