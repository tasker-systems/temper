/**
 * GET /auth/login — start the Auth0 OIDC PKCE flow.
 *
 * Generates a fresh CSRF state token and PKCE code verifier, stashes both
 * (plus the post-login `returnTo` destination) in a short-lived encrypted
 * cookie, then 302s to Auth0's /authorize endpoint. /auth/callback consumes
 * the cookie to verify state and exchange the code for tokens.
 */

import type { RequestHandler } from './$types';
import { redirect } from '@sveltejs/kit';
import { randomBytes, createHash } from 'node:crypto';
import { authorizeUrl } from '$lib/server/auth0';
import { writePkce } from '$lib/server/session';

function base64url(buf: Buffer): string {
	return buf.toString('base64').replace(/=/g, '').replace(/\+/g, '-').replace(/\//g, '_');
}

export const GET: RequestHandler = async ({ url, cookies, locals }) => {
	// If already authenticated, send the user where they were headed.
	const returnTo = url.searchParams.get('returnTo') ?? '/dashboard';
	if (locals.user) {
		throw redirect(303, returnTo);
	}

	const state = base64url(randomBytes(32));
	const verifier = base64url(randomBytes(32));
	const challenge = base64url(createHash('sha256').update(verifier).digest());

	await writePkce(cookies, { state, verifier, returnTo });

	throw redirect(302, authorizeUrl(state, challenge));
};
