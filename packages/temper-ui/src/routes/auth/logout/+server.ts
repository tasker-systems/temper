/**
 * GET /auth/logout — clear the local session cookie and redirect to Auth0's
 * /v2/logout so the user is signed out at the IdP as well as in temper.
 *
 * Auth0 will redirect back to APP_URL after logout (must be in the Allowed
 * Logout URLs list in the temper-web application config).
 */

import type { RequestHandler } from './$types';
import { redirect } from '@sveltejs/kit';
import { logoutUrl, APP_BASE_URL } from '$lib/server/auth0';
import { clearSession } from '$lib/server/session';

export const GET: RequestHandler = async ({ cookies }) => {
	clearSession(cookies);
	throw redirect(302, logoutUrl(APP_BASE_URL));
};
