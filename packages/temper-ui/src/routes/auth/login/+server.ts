/**
 * GET /auth/login — start the OIDC Authorization Code + PKCE flow.
 *
 * Generates a fresh CSRF state token and PKCE code verifier, stashes both
 * (plus the post-login `returnTo` destination) in a short-lived encrypted
 * cookie, then 302s to the provider's /authorize endpoint. /auth/callback
 * consumes the cookie to verify state and exchange the code for tokens.
 */

import { createHash, randomBytes } from 'node:crypto';
import { redirect } from '@sveltejs/kit';
import { authorizeUrl } from '$lib/server/oidc';
import { safeReturnTo } from '$lib/server/return-to';
import { writePkce } from '$lib/server/session';
import type { RequestHandler } from './$types';

function base64url(buf: Buffer): string {
	return buf.toString('base64').replace(/=/g, '').replace(/\+/g, '-').replace(/\//g, '_');
}

export const GET: RequestHandler = async ({ url, cookies, locals }) => {
	// Where the reader was headed — honoured only as a path on this site (`safeReturnTo`), both
	// for an already-signed-in reader and for the value carried through the provider round-trip.
	const returnTo = safeReturnTo(url.searchParams.get('returnTo'), url.origin);
	if (locals.user) {
		throw redirect(303, returnTo);
	}

	const state = base64url(randomBytes(32));
	const verifier = base64url(randomBytes(32));
	const challenge = base64url(createHash('sha256').update(verifier).digest());

	await writePkce(cookies, { state, verifier, returnTo });

	throw redirect(302, await authorizeUrl(state, challenge));
};
