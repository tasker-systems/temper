/**
 * Request middleware for every SvelteKit request.
 *
 * Before anything else, browser-facing API/MCP/OAuth/discovery paths are
 * reverse-proxied to the operator-configurable upstream (see `proxy.ts`) — this
 * replaces the old hardcoded `vercel.json` rewrites so self-hosted installs can
 * point those paths at their own API host purely through env. All other requests
 * fall through to session hydration below.
 *
 * Session hydration pipeline:
 *   1. Read the encrypted session cookie. No cookie → unauthenticated locals
 *      and return.
 *   2. If the access_token is within REFRESH_THRESHOLD_SECONDS of expiring,
 *      refresh it via the OIDC provider and persist the rotated tokens.
 *   3. Populate `locals.user` from the cached id_token claims and
 *      `locals.accessToken` from the (possibly refreshed) access_token.
 *   4. Fetch the temper profile via `/api/profile`. This is the source of
 *      truth for `entitlements` (system_access, is_admin) and is intentionally
 *      called on every request so admin/system_access changes propagate
 *      without requiring re-login. The call is cheap and the profile is
 *      hot in pgbouncer.
 *
 * On errors fetching the profile we log and leave `locals.profile` /
 * `locals.entitlements` null — downstream layout loads will redirect to
 * /auth/login or /request-access as appropriate.
 */

// Register OTLP span export before anything else runs. Side-effecting by design;
// a no-op when no OTLP endpoint is configured (local dev, endpoint-less installs).
import '$lib/server/telemetry/register';

import { type Handle, text } from '@sveltejs/kit';
import { ApiError, apiGet } from '$lib/server/api';
import { CSRF_FORBIDDEN_MESSAGE, isForbiddenCrossOriginFormPost } from '$lib/server/csrf';
import { REFRESH_THRESHOLD_SECONDS, refreshAccessToken } from '$lib/server/oidc';
import { isProxiedPath, proxyRequest } from '$lib/server/proxy';
import { clearSession, readSession, writeSession } from '$lib/server/session';
import { traceRequest } from '$lib/server/telemetry/request-span';
import type { ProfileWithEntitlements } from '$lib/types';

// The exported hook wraps the whole request in a server span (parented on the inbound
// `traceparent`) and runs the body inside its active context, so every outbound fetch —
// the reverse proxy AND the SSR data loaders — propagates the span. The span is opened
// around `handleRequest` in its entirety, before the proxy short-circuit, because the
// browser→ui→api acceptance hop returns early from the proxy and never reaches `resolve`.
export const handle: Handle = (input) => traceRequest(input.event, () => handleRequest(input));

const handleRequest = async ({ event, resolve }: Parameters<Handle>[0]): Promise<Response> => {
	// Reverse-proxy the API/MCP/OAuth/discovery surface to the upstream host
	// before any SvelteKit handling. These paths are not UI routes. This
	// includes the SAML ACS (`/oauth/saml/acs`), which by design takes a
	// cross-origin form POST from the IdP — authenticated by the SAML layer,
	// not by origin — so it must forward upstream before the CSRF guard below.
	if (isProxiedPath(event.url.pathname)) {
		return proxyRequest(event);
	}

	// SvelteKit's built-in `checkOrigin` is disabled (svelte.config.js) so the
	// proxied ACS POST above isn't blocked pre-hook. Re-apply the equivalent
	// origin guard here, scoped to the UI's own routes.
	if (isForbiddenCrossOriginFormPost(event.request, event.url.origin)) {
		return text(CSRF_FORBIDDEN_MESSAGE, { status: 403 });
	}

	event.locals.user = null;
	event.locals.accessToken = null;
	event.locals.profile = null;
	event.locals.entitlements = null;

	const session = await readSession(event.cookies);
	if (!session) {
		return resolve(event);
	}

	let { accessToken, refreshToken, idTokenClaims, expiresAt } = session;

	const nowSeconds = Math.floor(Date.now() / 1000);
	if (expiresAt - nowSeconds < REFRESH_THRESHOLD_SECONDS) {
		if (!refreshToken) {
			// No refresh token — session is unrecoverable. Drop it.
			clearSession(event.cookies);
			return resolve(event);
		}
		try {
			const tokens = await refreshAccessToken(refreshToken);
			accessToken = tokens.access_token;
			refreshToken = tokens.refresh_token ?? refreshToken;
			expiresAt = Math.floor(Date.now() / 1000) + tokens.expires_in;
			await writeSession(event.cookies, {
				accessToken,
				refreshToken,
				idTokenClaims,
				expiresAt,
			});
		} catch (err) {
			console.error('OIDC token refresh failed; clearing session', err);
			clearSession(event.cookies);
			return resolve(event);
		}
	}

	event.locals.accessToken = accessToken;
	event.locals.user = {
		sub: idTokenClaims.sub,
		email: idTokenClaims.email ?? null,
		name: idTokenClaims.name ?? null,
		picture: idTokenClaims.picture ?? null,
	};

	try {
		const profileResponse = await apiGet<ProfileWithEntitlements>('/api/profile', accessToken);
		const { entitlements, ...profile } = profileResponse;
		event.locals.profile = profile;
		event.locals.entitlements = entitlements;
	} catch (err) {
		if (err instanceof ApiError && err.status === 401) {
			// Token rejected by API — session is dead. Drop it and let the
			// downstream load run unauthenticated; it will redirect to /auth/login.
			clearSession(event.cookies);
		} else {
			console.error('Failed to fetch /api/profile in hooks.server.ts', err);
		}
	}

	return resolve(event);
};
