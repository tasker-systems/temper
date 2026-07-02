/**
 * GET /auth/callback — the OIDC provider redirects here after the user
 * completes login.
 *
 * Validates the CSRF `state` parameter against the value stashed in the PKCE
 * cookie, exchanges the authorization code for tokens, and writes an
 * encrypted session cookie before redirecting to the user's original
 * destination (or /vault/all).
 *
 * On any error in the exchange (state mismatch, expired code, provider down),
 * we send the user to /?error=auth_failed rather than throwing — the user
 * shouldn't see a stack trace just because they took too long on the login
 * page.
 */

import type { RequestHandler } from './$types';
import { redirect } from '@sveltejs/kit';
import { exchangeCode, identityClaimsFromTokens } from '$lib/server/oidc';
import { readPkce, clearPkce, writeSession } from '$lib/server/session';

export const GET: RequestHandler = async ({ url, cookies }) => {
	const code = url.searchParams.get('code');
	const state = url.searchParams.get('state');
	const error = url.searchParams.get('error');

	if (error) {
		console.warn('OIDC callback returned error', { error, description: url.searchParams.get('error_description') });
		clearPkce(cookies);
		throw redirect(303, '/?error=auth_failed');
	}

	if (!code || !state) {
		clearPkce(cookies);
		throw redirect(303, '/?error=auth_missing_params');
	}

	const pkce = await readPkce(cookies);
	if (!pkce) {
		throw redirect(303, '/?error=auth_state_lost');
	}

	if (pkce.state !== state) {
		console.warn('OIDC callback state mismatch — possible CSRF', {
			expected: pkce.state,
			received: state
		});
		clearPkce(cookies);
		throw redirect(303, '/?error=auth_state_mismatch');
	}

	let tokens;
	try {
		tokens = await exchangeCode(code, pkce.verifier);
	} catch (err) {
		console.error('OIDC token exchange failed', err);
		clearPkce(cookies);
		throw redirect(303, '/?error=auth_exchange_failed');
	}

	let idTokenClaims;
	try {
		idTokenClaims = identityClaimsFromTokens(tokens);
	} catch (err) {
		console.error('OIDC identity decode failed', err);
		clearPkce(cookies);
		throw redirect(303, '/?error=auth_exchange_failed');
	}

	await writeSession(cookies, {
		accessToken: tokens.access_token,
		refreshToken: tokens.refresh_token ?? null,
		idTokenClaims,
		expiresAt: Math.floor(Date.now() / 1000) + tokens.expires_in
	});

	clearPkce(cookies);

	// SvelteKit's `redirect` throws — must be outside try/catch above.
	throw redirect(303, pkce.returnTo);
};
