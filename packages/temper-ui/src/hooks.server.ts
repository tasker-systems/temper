/**
 * Request middleware that hydrates `event.locals` for every SvelteKit request.
 *
 * Pipeline:
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

import type { Handle } from '@sveltejs/kit';
import { readSession, writeSession, clearSession } from '$lib/server/session';
import { refreshAccessToken, REFRESH_THRESHOLD_SECONDS } from '$lib/server/oidc';
import { apiGet, ApiError } from '$lib/server/api';
import type { ProfileWithEntitlements } from '$lib/types';

export const handle: Handle = async ({ event, resolve }) => {
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
				expiresAt
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
		picture: idTokenClaims.picture ?? null
	};

	try {
		const profileResponse = await apiGet<ProfileWithEntitlements>(
			'/api/profile',
			accessToken
		);
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
