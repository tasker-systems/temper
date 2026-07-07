/**
 * Origin-based CSRF guard for the UI's own routes.
 *
 * SvelteKit ships a blanket `csrf.checkOrigin` check that rejects any
 * mutating form `POST`/`PUT`/`PATCH`/`DELETE` whose `Origin` differs from the
 * app origin. That check runs *ahead* of the `handle` hook, so it fires before
 * the reverse-proxy short-circuit in `hooks.server.ts` can forward a request
 * upstream. On the self-hosted SAML path the IdP delivers the assertion as a
 * browser-submitted form `POST` to `/oauth/saml/acs` from the IdP's origin —
 * a legitimately cross-origin POST that the ACS is *designed* to receive
 * (authenticated by the SAML layer: signature, audience, destination, replay
 * guard — far stronger than an `Origin` match). The built-in check turns that
 * into `403 Cross-site POST form submissions are forbidden` before the proxy
 * ever runs.
 *
 * So we disable `checkOrigin` in `svelte.config.js` and re-implement the
 * equivalent guard here, applied only *after* the proxied surface (including
 * the ACS) has been short-circuited. The UI's own form actions keep their
 * origin protection; the proxied ACS POST is exempt because the API's SAML
 * validation is the real protection there.
 *
 * The predicate is pure so it can be unit-tested without a running server; the
 * hook applies it and returns the 403 response.
 */

const MUTATING_METHODS = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);

/**
 * Content types a browser can send from an HTML form without a preflight —
 * the ones a cross-site form-submission CSRF attack can actually forge. This
 * mirrors SvelteKit's built-in check (and the CORS "simple request" set).
 */
const FORM_CONTENT_TYPES = new Set([
	'application/x-www-form-urlencoded',
	'multipart/form-data',
	'text/plain'
]);

/** The message SvelteKit's built-in check returns; preserved for parity. */
export const CSRF_FORBIDDEN_MESSAGE = 'Cross-site POST form submissions are forbidden';

/** Extract the bare media type from a `Content-Type` header (drop params, normalize case). */
function mediaType(contentType: string | null): string {
	return (contentType ?? '').split(';')[0].trim().toLowerCase();
}

/** Whether a request carries a browser-forgeable form body. */
function isFormSubmission(request: Request): boolean {
	return FORM_CONTENT_TYPES.has(mediaType(request.headers.get('content-type')));
}

/**
 * Whether a request is a cross-origin form submission that the origin CSRF
 * guard should reject: a mutating method, a form-encoded body, and an `Origin`
 * header that does not match the app origin. A missing `Origin` never matches
 * `appOrigin`, so it is treated as cross-origin (same as SvelteKit's default).
 */
export function isForbiddenCrossOriginFormPost(request: Request, appOrigin: string): boolean {
	return (
		MUTATING_METHODS.has(request.method) &&
		isFormSubmission(request) &&
		request.headers.get('origin') !== appOrigin
	);
}
