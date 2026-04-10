/**
 * Server load for the authenticated `(app)` layout group.
 *
 * Two-step gate:
 *   1. No session → /auth/login?returnTo=<current-path>
 *   2. Session exists but `entitlements.system_access === false`
 *      → /request-access (where the user submits a join request)
 *
 * If both checks pass, expose `user`, `profile`, and `entitlements` to the
 * layout component (and to all child page loads via `parent()`).
 */

import type { LayoutServerLoad } from './$types';
import { redirect } from '@sveltejs/kit';
import { apiGet } from '$lib/server/api';
import type { ContextRowWithCounts } from '$lib/types';

export const load: LayoutServerLoad = async ({ locals, url }) => {
	if (!locals.user || !locals.accessToken) {
		const returnTo = encodeURIComponent(url.pathname + url.search);
		throw redirect(303, `/auth/login?returnTo=${returnTo}`);
	}

	if (!locals.profile || !locals.entitlements) {
		// Auth succeeded but the API call to /api/profile failed in
		// hooks.server.ts. Treat as a transient failure and bounce to login.
		throw redirect(303, '/auth/login');
	}

	if (!locals.entitlements.system_access) {
		throw redirect(303, '/request-access');
	}

	const contexts = await apiGet<ContextRowWithCounts[]>(
		'/api/contexts',
		locals.accessToken!
	).catch(() => [] as ContextRowWithCounts[]);

	return {
		user: locals.user,
		profile: locals.profile,
		entitlements: locals.entitlements,
		contexts
	};
};
