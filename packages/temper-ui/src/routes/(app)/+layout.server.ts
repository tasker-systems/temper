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
import type { ContextRowWithCounts, PublicSystemSettings } from '$lib/types';

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

	// Instance branding ("temper @ acme") is operator-configured via the
	// DB-backed system settings; a self-hosted org sets `instance_name`. A null
	// value (or a failed fetch) falls back to the default wordmark in the shell.
	const [contexts, settings] = await Promise.all([
		apiGet<ContextRowWithCounts[]>('/api/contexts', locals.accessToken!).catch(
			() => [] as ContextRowWithCounts[]
		),
		apiGet<PublicSystemSettings>('/api/access/settings', locals.accessToken!).catch(() => null)
	]);

	return {
		user: locals.user,
		profile: locals.profile,
		entitlements: locals.entitlements,
		contexts,
		instanceName: settings?.instance_name ?? null
	};
};
