/**
 * Provider-agnostic OIDC core — pure helpers with no `$env` or network access.
 *
 * Everything here is a pure function so it can be unit-tested directly. The
 * env-bound wiring (reading config, fetching the discovery document, memoising
 * it, and exposing the public async API) lives in `oidc.ts`, which composes
 * these helpers.
 *
 * This realises the Authorization Code + PKCE flow generically: endpoints come
 * from OIDC discovery (`/.well-known/openid-configuration`) rather than being
 * hardcoded to one provider, and the only provider-specific knob is the
 * optional `audience` param (Auth0 needs it; Okta custom auth servers carry it
 * implicitly).
 */

/** Resolved, validated OIDC configuration. */
export interface OidcConfig {
	issuer: string;
	clientId: string;
	/**
	 * Optional — a public PKCE client (e.g. the Temper AS, which advertises
	 * `token_endpoint_auth_methods_supported: ["none"]`) has no secret; PKCE
	 * (code_verifier/code_challenge) is the security, not a shared secret.
	 * Confidential clients (Auth0 Regular Web App) still supply one.
	 */
	clientSecret?: string;
	/** Optional — Auth0 needs it; most providers don't. */
	audience?: string;
	/**
	 * Optional discovery-document URL override. Defaults to
	 * `${issuer}/.well-known/openid-configuration`; set this when the provider
	 * serves metadata elsewhere, e.g. the Temper AS's RFC 8414 endpoint at
	 * `/.well-known/oauth-authorization-server`.
	 */
	discoveryUrl?: string;
}

/** The subset of the discovery document we consume. */
export interface OidcEndpoints {
	authorization_endpoint: string;
	token_endpoint: string;
	/** Absent when the provider has no RP-initiated logout. */
	end_session_endpoint?: string;
	jwks_uri?: string;
}

/**
 * Token endpoint response (authorization_code and refresh_token grants).
 *
 * `id_token` is optional: full-OIDC providers (Auth0) return one, but the
 * Temper AS is OAuth-only and returns `{ access_token, token_type,
 * expires_in, refresh_token }` with no `id_token` — see
 * `identityClaimsFromTokens`, which falls back to the access_token in that
 * case.
 */
export interface OidcTokenResponse {
	access_token: string;
	id_token?: string;
	refresh_token?: string;
	expires_in: number;
	token_type: string;
	scope?: string;
}

export interface OidcIdTokenClaims {
	sub: string;
	email?: string;
	email_verified?: boolean;
	name?: string;
	picture?: string;
	exp: number;
	iat: number;
	[key: string]: unknown;
}

type EnvLike = Record<string, string | undefined>;

/**
 * Resolve OIDC config from an env-like record, preferring `OIDC_*` keys and
 * falling back to the canonical deployment's `AUTH0_*` keys. The fallback lets
 * the existing temperkb.io Vercel project keep working with zero env changes
 * while self-hosters configure a different issuer purely through `OIDC_*`.
 */
export function resolveOidcConfig(env: EnvLike): OidcConfig {
	const rawIssuer =
		env.OIDC_ISSUER ?? (env.AUTH0_DOMAIN ? `https://${env.AUTH0_DOMAIN}` : undefined);
	const clientId = env.OIDC_CLIENT_ID ?? env.AUTH0_CLIENT_ID;
	const clientSecret = env.OIDC_CLIENT_SECRET ?? env.AUTH0_CLIENT_SECRET;
	const audience = env.OIDC_AUDIENCE ?? env.AUTH0_AUDIENCE;
	const rawDiscoveryUrl = env.OIDC_DISCOVERY_URL;
	const isPublicClient = /^(true|1|yes|on)$/i.test((env.OIDC_PUBLIC_CLIENT ?? '').trim());

	if (!rawIssuer) {
		throw new Error('OIDC issuer not configured: set OIDC_ISSUER (or AUTH0_DOMAIN)');
	}
	if (!clientId) {
		throw new Error('OIDC client id not configured: set OIDC_CLIENT_ID (or AUTH0_CLIENT_ID)');
	}
	// clientSecret is optional only for a declared public PKCE client (e.g. the
	// Temper AS) — PKCE alone secures the code exchange there. For a
	// confidential client (Auth0 Regular Web App), a missing secret must fail
	// fast here rather than surface later as an opaque 401 from the provider.
	if (!clientSecret && !isPublicClient) {
		throw new Error(
			'OIDC client secret not configured: set OIDC_CLIENT_SECRET (or AUTH0_CLIENT_SECRET), or set OIDC_PUBLIC_CLIENT=true for a public PKCE client'
		);
	}

	return {
		issuer: rawIssuer.replace(/\/$/, ''),
		clientId,
		clientSecret: clientSecret ? clientSecret : undefined,
		audience: audience ? audience : undefined,
		discoveryUrl: rawDiscoveryUrl ? rawDiscoveryUrl.replace(/\/$/, '') : undefined
	};
}

/**
 * Validate and narrow a discovery document to the endpoints we use. Throws if a
 * required endpoint is missing so misconfiguration fails loudly at first use.
 */
export function parseDiscovery(payload: unknown): OidcEndpoints {
	if (typeof payload !== 'object' || payload === null) {
		throw new Error('OIDC discovery document is not a JSON object');
	}
	const doc = payload as Record<string, unknown>;

	const authorization_endpoint = doc.authorization_endpoint;
	const token_endpoint = doc.token_endpoint;
	if (typeof authorization_endpoint !== 'string') {
		throw new Error('OIDC discovery document missing authorization_endpoint');
	}
	if (typeof token_endpoint !== 'string') {
		throw new Error('OIDC discovery document missing token_endpoint');
	}

	const end_session_endpoint =
		typeof doc.end_session_endpoint === 'string' ? doc.end_session_endpoint : undefined;
	const jwks_uri = typeof doc.jwks_uri === 'string' ? doc.jwks_uri : undefined;

	return { authorization_endpoint, token_endpoint, end_session_endpoint, jwks_uri };
}

/** Params needed to build an /authorize redirect, independent of provider. */
export interface AuthorizeParams {
	clientId: string;
	redirectUri: string;
	audience?: string;
}

/**
 * Build the Authorization Code + PKCE `/authorize` URL. `state` is a CSRF
 * token; `codeChallenge` is the SHA-256 hash (base64url) of the PKCE verifier
 * the caller stores until the callback runs.
 */
export function buildAuthorizeUrl(
	authorizationEndpoint: string,
	params: AuthorizeParams,
	state: string,
	codeChallenge: string
): string {
	const search = new URLSearchParams({
		response_type: 'code',
		client_id: params.clientId,
		redirect_uri: params.redirectUri,
		// Hardcoded rather than provider-aware: the Temper AS's temper-as preset
		// declares ["openid","offline_access"] but the AS currently ignores the
		// `scope` param entirely, so this full set is harmless-for-now on both
		// providers. Revisit if the AS starts enforcing requested scopes.
		scope: 'openid profile email offline_access',
		state,
		code_challenge: codeChallenge,
		code_challenge_method: 'S256'
	});
	if (params.audience) {
		search.set('audience', params.audience);
	}
	return `${authorizationEndpoint}?${search.toString()}`;
}

/** Params needed for an RP-initiated logout. */
export interface LogoutParams {
	clientId: string;
	returnTo: string;
	idToken?: string;
}

/**
 * Build a standard RP-initiated logout URL from `end_session_endpoint`. When
 * the provider advertises no `end_session_endpoint`, there is nothing to redirect
 * through — return `returnTo` directly so the caller still lands the user home.
 */
export function buildLogoutUrl(
	endSessionEndpoint: string | undefined,
	params: LogoutParams
): string {
	if (!endSessionEndpoint) {
		return params.returnTo;
	}
	const search = new URLSearchParams({
		post_logout_redirect_uri: params.returnTo,
		client_id: params.clientId
	});
	if (params.idToken) {
		search.set('id_token_hint', params.idToken);
	}
	return `${endSessionEndpoint}?${search.toString()}`;
}

/**
 * Decode an id_token's claims WITHOUT verifying the signature.
 *
 * The id_token arrives directly from the provider's token endpoint over TLS in
 * a server-to-server call, so the source is trusted and we only need the claims
 * to populate `locals.user`. This is distinct from the Rust API's JWKS-based
 * verification of the access_token, which DOES authenticate a token that
 * arrived over the wire.
 */
export function decodeIdToken(idToken: string): OidcIdTokenClaims {
	const parts = idToken.split('.');
	if (parts.length !== 3) {
		throw new Error('Invalid id_token: expected 3 segments');
	}
	const payload = parts[1];
	const padded = payload + '='.repeat((4 - (payload.length % 4)) % 4);
	const b64 = padded.replace(/-/g, '+').replace(/_/g, '/');
	const json = Buffer.from(b64, 'base64').toString('utf-8');
	return JSON.parse(json) as OidcIdTokenClaims;
}

/**
 * Resolve identity claims from a token response. Full-OIDC providers (Auth0) return an id_token;
 * the Temper AS is OAuth-only and returns no id_token, but its access_token is an EdDSA JWT that
 * already carries sub/email/email_verified — fall back to decoding that.
 */
export function identityClaimsFromTokens(tokens: OidcTokenResponse): OidcIdTokenClaims {
	const token = tokens.id_token ?? tokens.access_token;
	return decodeIdToken(token);
}
