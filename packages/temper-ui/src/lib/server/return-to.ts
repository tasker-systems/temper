/**
 * The one check on where sign-in sends a person afterwards.
 *
 * `returnTo` rides the query string of `/auth/login`, so anyone can write it into a link. It is
 * honoured only when it names a path on this site; anything else — an absolute URL, a
 * scheme-relative `//host`, `/\host`, a value carrying characters a browser strips or normalises
 * (tab, CR/LF, other controls, space, backslash), or one that does not start with `/` — lands on
 * `ARRIVAL` instead. `/auth/login` calls this before redirecting a signed-in reader and before
 * stashing the value in the PKCE cookie; `/auth/callback` calls it again before redirecting, so a
 * cookie written by an older build is not trusted to hold a checked value.
 *
 * The leading-character check and the parse are both required: the parse catches what a browser
 * would resolve off-site, and the normalised result is re-checked because dot-segment removal can
 * turn a harmless-looking path (`/.//host`) into a scheme-relative one.
 */

import { ARRIVAL } from '$lib/first-view';

/** A same-origin path: one leading `/`, never followed by `/` or `\`. */
const SITE_PATH = /^\/(?![/\\])/;

/**
 * C0 controls, space and DEL anywhere — a URL parser strips or folds them — and a backslash in the
 * path, where an http(s) parser reads it as `/`. A backslash after `?` or `#` is left alone: the
 * parser keeps it literal there, and a hand-typed `/vault?q=C:\path` should still come back.
 */
function hasUnsafeChar(value: string): boolean {
	let inPath = true;
	for (let i = 0; i < value.length; i++) {
		const code = value.charCodeAt(i);
		if (code === 0x3f || code === 0x23) inPath = false;
		if (code <= 0x20 || code === 0x7f || (inPath && code === 0x5c)) return true;
	}
	return false;
}

export function safeReturnTo(value: string | null | undefined, origin: string): string {
	if (typeof value !== 'string' || !SITE_PATH.test(value) || hasUnsafeChar(value)) {
		return ARRIVAL;
	}
	let parsed: URL;
	try {
		parsed = new URL(value, origin);
	} catch {
		return ARRIVAL;
	}
	if (parsed.origin !== new URL(origin).origin) return ARRIVAL;
	const path = `${parsed.pathname}${parsed.search}${parsed.hash}`;
	return SITE_PATH.test(path) ? path : ARRIVAL;
}
