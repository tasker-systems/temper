/**
 * Auth0 OIDC server-side helpers.
 *
 * Implements the Authorization Code + PKCE flow for the temper-web Regular
 * Web Application registered in the `temperkb.us.auth0.com` tenant. The flow
 * is:
 *
 *   1. /auth/login generates a state + PKCE verifier, stashes them in a
 *      short-lived cookie, and redirects to `authorizeUrl()`.
 *   2. Auth0 sends the user back to /auth/callback with `code` + `state`.
 *   3. The callback verifies state, calls `exchangeCode()`, and writes the
 *      session cookie via `lib/server/session.ts`.
 *   4. `hooks.server.ts` calls `refreshAccessToken()` when the cached token
 *      is within REFRESH_THRESHOLD_SECONDS of expiring.
 *
 * All env vars are read at module-eval time so the SvelteKit server fails
 * fast on startup if any are missing.
 */

import {
	AUTH0_DOMAIN,
	AUTH0_CLIENT_ID,
	AUTH0_CLIENT_SECRET,
	AUTH0_AUDIENCE,
	APP_URL
} from '$env/static/private';

export const REFRESH_THRESHOLD_SECONDS = 60;

export interface Auth0TokenResponse {
	access_token: string;
	id_token: string;
	refresh_token?: string;
	expires_in: number;
	token_type: string;
	scope?: string;
}

export interface Auth0IdTokenClaims {
	sub: string;
	email?: string;
	email_verified?: boolean;
	name?: string;
	picture?: string;
	exp: number;
	iat: number;
	[key: string]: unknown;
}

const AUTH0_ISSUER = `https://${AUTH0_DOMAIN}`;
export const AUTH0_REDIRECT_URI = `${APP_URL}/auth/callback`;
export const APP_BASE_URL = APP_URL;

/**
 * Build the Auth0 /authorize URL for the start of the OIDC flow.
 *
 * `state` is a CSRF token (random). `codeChallenge` is the SHA-256 hash of
 * the verifier the caller will store in a cookie until the callback runs.
 */
export function authorizeUrl(state: string, codeChallenge: string): string {
	const params = new URLSearchParams({
		response_type: 'code',
		client_id: AUTH0_CLIENT_ID,
		redirect_uri: AUTH0_REDIRECT_URI,
		scope: 'openid profile email offline_access',
		audience: AUTH0_AUDIENCE,
		state,
		code_challenge: codeChallenge,
		code_challenge_method: 'S256'
	});
	return `${AUTH0_ISSUER}/authorize?${params.toString()}`;
}

/**
 * Build the Auth0 /v2/logout URL. After logout, Auth0 redirects the user
 * back to `returnTo` (which must be in the Allowed Logout URLs list in the
 * Auth0 dashboard).
 */
export function logoutUrl(returnTo: string): string {
	const params = new URLSearchParams({
		client_id: AUTH0_CLIENT_ID,
		returnTo
	});
	return `${AUTH0_ISSUER}/v2/logout?${params.toString()}`;
}

/**
 * Exchange the authorization code received at /auth/callback for tokens.
 * The `codeVerifier` must be the plaintext PKCE verifier that produced the
 * `code_challenge` originally sent to /authorize.
 */
export async function exchangeCode(
	code: string,
	codeVerifier: string
): Promise<Auth0TokenResponse> {
	const res = await fetch(`${AUTH0_ISSUER}/oauth/token`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
		body: new URLSearchParams({
			grant_type: 'authorization_code',
			client_id: AUTH0_CLIENT_ID,
			client_secret: AUTH0_CLIENT_SECRET,
			code,
			redirect_uri: AUTH0_REDIRECT_URI,
			code_verifier: codeVerifier
		})
	});

	if (!res.ok) {
		const body = await res.text().catch(() => '');
		throw new Error(`Auth0 token exchange failed (${res.status}): ${body}`);
	}

	return res.json() as Promise<Auth0TokenResponse>;
}

/**
 * Use the cached refresh_token to get a fresh access_token + id_token.
 * Auth0 may rotate the refresh token; the caller must persist whichever
 * one comes back in the response.
 */
export async function refreshAccessToken(refreshToken: string): Promise<Auth0TokenResponse> {
	const res = await fetch(`${AUTH0_ISSUER}/oauth/token`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
		body: new URLSearchParams({
			grant_type: 'refresh_token',
			client_id: AUTH0_CLIENT_ID,
			client_secret: AUTH0_CLIENT_SECRET,
			refresh_token: refreshToken
		})
	});

	if (!res.ok) {
		const body = await res.text().catch(() => '');
		throw new Error(`Auth0 token refresh failed (${res.status}): ${body}`);
	}

	return res.json() as Promise<Auth0TokenResponse>;
}

/**
 * Decode an Auth0 id_token without signature verification.
 *
 * We do NOT verify the signature here because the id_token came directly
 * from the Auth0 token endpoint over TLS in a server-to-server call — the
 * source is trusted. We only need the claims to populate `locals.user`.
 *
 * This is distinct from the Rust API's JWKS-based verification of the
 * access_token, which IS necessary because that token arrives over the
 * wire and must be authenticated.
 */
export function decodeIdToken(idToken: string): Auth0IdTokenClaims {
	const parts = idToken.split('.');
	if (parts.length !== 3) {
		throw new Error('Invalid id_token: expected 3 segments');
	}
	const payload = parts[1];
	// base64url → base64 → JSON
	const padded = payload + '='.repeat((4 - (payload.length % 4)) % 4);
	const b64 = padded.replace(/-/g, '+').replace(/_/g, '/');
	const json = Buffer.from(b64, 'base64').toString('utf-8');
	return JSON.parse(json) as Auth0IdTokenClaims;
}
