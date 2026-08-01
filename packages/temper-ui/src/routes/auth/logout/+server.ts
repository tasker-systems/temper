/**
 * GET /auth/logout — clear the local session cookie and redirect to the OIDC
 * provider's RP-initiated logout (`end_session_endpoint`) so the user is signed
 * out at the IdP as well as in temper.
 *
 * The provider redirects back to APP_URL after logout, so APP_URL must be a
 * registered post-logout redirect URI (e.g. Auth0's Allowed Logout URLs).
 */

import { redirect } from '@sveltejs/kit';
import { APP_BASE_URL, logoutUrl } from '$lib/server/oidc';
import { clearSession } from '$lib/server/session';
import type { RequestHandler } from './$types';

export const GET: RequestHandler = async ({ cookies }) => {
	clearSession(cookies);
	throw redirect(302, await logoutUrl(APP_BASE_URL));
};
