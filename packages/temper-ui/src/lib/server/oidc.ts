/**
 * Provider-agnostic OIDC server-side helpers (Authorization Code + PKCE).
 *
 * Endpoints are resolved at runtime from the issuer's discovery document,
 * fetched by default from `/.well-known/openid-configuration` (or from
 * `OIDC_DISCOVERY_URL` when set — e.g. a Temper AS instance serves RFC 8414
 * metadata at `/.well-known/oauth-authorization-server`), so any OIDC/OAuth
 * provider works — the canonical deployment runs against Auth0, a self-hosted
 * install can point at a Temper AS, Okta, Keycloak, etc. purely through env.
 * Config is read from `OIDC_*` keys with a fallback to the canonical
 * deployment's existing `AUTH0_*` keys (see `resolveOidcConfig`), so
 * temperkb.io keeps working with no env changes.
 *
 * The flow:
 *   1. /auth/login generates state + PKCE verifier, stashes them in a
 *      short-lived cookie, and redirects to `authorizeUrl()`.
 *   2. The provider sends the user back to /auth/callback with `code` + `state`.
 *   3. The callback verifies state, calls `exchangeCode()`, and writes the
 *      session cookie via `lib/server/session.ts`.
 *   4. `hooks.server.ts` calls `refreshAccessToken()` when the cached token is
 *      within REFRESH_THRESHOLD_SECONDS of expiring.
 *
 * Pure helpers (config resolution, URL building, token decode) live in
 * `oidc-core.ts` and are unit-tested directly; this module owns the env read,
 * the discovery fetch + memoisation, and the public async API.
 */

import { env } from '$env/dynamic/private';
import {
	resolveOidcConfig,
	parseDiscovery,
	buildAuthorizeUrl,
	buildLogoutUrl,
	decodeIdToken,
	identityClaimsFromTokens,
	type OidcConfig,
	type OidcEndpoints,
	type OidcTokenResponse,
	type OidcIdTokenClaims
} from './oidc-core';

export { decodeIdToken, identityClaimsFromTokens };
export type { OidcTokenResponse, OidcIdTokenClaims };

export const REFRESH_THRESHOLD_SECONDS = 60;

const config: OidcConfig = resolveOidcConfig(env);
export const APP_BASE_URL = env.APP_URL ?? '';
export const OIDC_REDIRECT_URI = `${APP_BASE_URL}/auth/callback`;

/**
 * Memoised discovery lookup. The document is immutable for the lifetime of the
 * server process, so we fetch it once and reuse the promise.
 */
let discoveryPromise: Promise<OidcEndpoints> | null = null;
export function discovery(): Promise<OidcEndpoints> {
	if (!discoveryPromise) {
		const url = config.discoveryUrl ?? `${config.issuer}/.well-known/openid-configuration`;
		discoveryPromise = fetch(url)
			.then(async (res) => {
				if (!res.ok) {
					throw new Error(`OIDC discovery failed (${res.status}) at ${url}`);
				}
				return parseDiscovery(await res.json());
			})
			.catch((err) => {
				// Don't cache a failed lookup — let the next request retry.
				discoveryPromise = null;
				throw err;
			});
	}
	return discoveryPromise;
}

/**
 * Build the `/authorize` URL for the start of the OIDC flow. `state` is a CSRF
 * token; `codeChallenge` is the SHA-256 hash of the PKCE verifier the caller
 * stores in a cookie until the callback runs.
 */
export async function authorizeUrl(state: string, codeChallenge: string): Promise<string> {
	const { authorization_endpoint } = await discovery();
	return buildAuthorizeUrl(
		authorization_endpoint,
		{ clientId: config.clientId, redirectUri: OIDC_REDIRECT_URI, audience: config.audience },
		state,
		codeChallenge
	);
}

/**
 * Build the RP-initiated logout URL. The provider redirects the user back to
 * `returnTo` after logout (which must be a registered post-logout redirect URI).
 * `idToken`, when supplied, is passed as `id_token_hint` per the spec.
 */
export async function logoutUrl(returnTo: string, idToken?: string): Promise<string> {
	const { end_session_endpoint } = await discovery();
	return buildLogoutUrl(end_session_endpoint, { clientId: config.clientId, returnTo, idToken });
}

/**
 * Exchange the authorization code received at /auth/callback for tokens. The
 * `codeVerifier` must be the plaintext PKCE verifier that produced the
 * `code_challenge` originally sent to /authorize.
 */
export async function exchangeCode(
	code: string,
	codeVerifier: string
): Promise<OidcTokenResponse> {
	const { token_endpoint } = await discovery();
	return tokenRequest(token_endpoint, {
		grant_type: 'authorization_code',
		client_id: config.clientId,
		...(config.clientSecret ? { client_secret: config.clientSecret } : {}),
		code,
		redirect_uri: OIDC_REDIRECT_URI,
		code_verifier: codeVerifier
	});
}

/**
 * Use the cached refresh_token to get a fresh access_token + id_token. The
 * provider may rotate the refresh token; the caller must persist whichever one
 * comes back in the response.
 */
export async function refreshAccessToken(refreshToken: string): Promise<OidcTokenResponse> {
	const { token_endpoint } = await discovery();
	return tokenRequest(token_endpoint, {
		grant_type: 'refresh_token',
		client_id: config.clientId,
		...(config.clientSecret ? { client_secret: config.clientSecret } : {}),
		refresh_token: refreshToken
	});
}

async function tokenRequest(
	endpoint: string,
	body: Record<string, string>
): Promise<OidcTokenResponse> {
	const res = await fetch(endpoint, {
		method: 'POST',
		headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
		body: new URLSearchParams(body)
	});
	if (!res.ok) {
		const detail = await res.text().catch(() => '');
		throw new Error(`OIDC token request failed (${res.status}): ${detail}`);
	}
	return res.json() as Promise<OidcTokenResponse>;
}
