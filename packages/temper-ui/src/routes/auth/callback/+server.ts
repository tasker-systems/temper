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

import { redirect } from '@sveltejs/kit';
import { exchangeCode, identityClaimsFromTokens } from '$lib/server/oidc';
import { clearPkce, readPkce, writeSession } from '$lib/server/session';
import type { RequestHandler } from './$types';

export const GET: RequestHandler = async ({ url, cookies }) => {
	const code = url.searchParams.get('code');
	const state = url.searchParams.get('state');
	const error = url.searchParams.get('error');

	if (error) {
		// All three values ride the request URL — caller-chosen content at
		// caller-chosen length. Bounded before they reach the log stream.
		console.warn('OIDC callback returned error', {
			error: error.slice(0, 128),
			description: url.searchParams.get('error_description')?.slice(0, 512),
		});
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
			// Both are nonce material: bounded, and identified by shape rather than
			// logged whole.
			expectedLen: pkce.state.length,
			received: state.slice(0, 128),
		});
		clearPkce(cookies);
		throw redirect(303, '/?error=auth_state_mismatch');
	}

	let tokens: Awaited<ReturnType<typeof exchangeCode>>;
	try {
		tokens = await exchangeCode(code, pkce.verifier);
	} catch (err) {
		console.error('OIDC token exchange failed', err);
		clearPkce(cookies);
		throw redirect(303, '/?error=auth_exchange_failed');
	}

	let idTokenClaims: ReturnType<typeof identityClaimsFromTokens>;
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
		expiresAt: Math.floor(Date.now() / 1000) + tokens.expires_in,
	});

	clearPkce(cookies);

	// SvelteKit's `redirect` throws — must be outside try/catch above.
	throw redirect(303, pkce.returnTo);
};
