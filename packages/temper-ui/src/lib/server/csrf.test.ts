import { describe, expect, it } from 'vitest';
import { isForbiddenCrossOriginFormPost } from './csrf';

const APP_ORIGIN = 'https://app.example.com';

/** Build a Request with the given method, content-type, and origin header. */
function req(
	method: string,
	contentType: string | null,
	origin: string | null,
	url = `${APP_ORIGIN}/some-action`
): Request {
	const headers = new Headers();
	if (contentType !== null) headers.set('content-type', contentType);
	if (origin !== null) headers.set('origin', origin);
	// Only attach a body for methods that permit one.
	const body = method === 'GET' || method === 'HEAD' ? undefined : 'a=1';
	return new Request(url, { method, headers, body });
}

describe('isForbiddenCrossOriginFormPost', () => {
	it('rejects a cross-origin form POST (the built-in CSRF case)', () => {
		expect(
			isForbiddenCrossOriginFormPost(
				req('POST', 'application/x-www-form-urlencoded', 'https://evil.example.com'),
				APP_ORIGIN
			)
		).toBe(true);
	});

	it('rejects cross-origin multipart and text/plain form posts too', () => {
		expect(
			isForbiddenCrossOriginFormPost(
				req('POST', 'multipart/form-data; boundary=x', 'https://evil.example.com'),
				APP_ORIGIN
			)
		).toBe(true);
		expect(
			isForbiddenCrossOriginFormPost(
				req('POST', 'text/plain', 'https://evil.example.com'),
				APP_ORIGIN
			)
		).toBe(true);
	});

	it('rejects all mutating methods, not just POST', () => {
		for (const method of ['POST', 'PUT', 'PATCH', 'DELETE']) {
			expect(
				isForbiddenCrossOriginFormPost(
					req(method, 'application/x-www-form-urlencoded', 'https://evil.example.com'),
					APP_ORIGIN
				)
			).toBe(true);
		}
	});

	it('treats a missing Origin header as cross-origin', () => {
		expect(
			isForbiddenCrossOriginFormPost(
				req('POST', 'application/x-www-form-urlencoded', null),
				APP_ORIGIN
			)
		).toBe(true);
	});

	it('allows a same-origin form POST (a legitimate UI form action)', () => {
		expect(
			isForbiddenCrossOriginFormPost(
				req('POST', 'application/x-www-form-urlencoded', APP_ORIGIN),
				APP_ORIGIN
			)
		).toBe(false);
	});

	it('ignores content-type parameters and casing when matching form types', () => {
		expect(
			isForbiddenCrossOriginFormPost(
				req('POST', 'Application/X-WWW-Form-Urlencoded; charset=UTF-8', 'https://evil.example.com'),
				APP_ORIGIN
			)
		).toBe(true);
	});

	it('allows a cross-origin non-form POST (e.g. application/json — CORS-preflighted, not forgeable)', () => {
		expect(
			isForbiddenCrossOriginFormPost(
				req('POST', 'application/json', 'https://evil.example.com'),
				APP_ORIGIN
			)
		).toBe(false);
	});

	it('allows GET regardless of origin (CSRF does not apply to safe methods)', () => {
		expect(
			isForbiddenCrossOriginFormPost(
				req('GET', 'application/x-www-form-urlencoded', 'https://evil.example.com'),
				APP_ORIGIN
			)
		).toBe(false);
	});
});
