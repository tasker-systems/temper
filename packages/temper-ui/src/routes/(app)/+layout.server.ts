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

import { redirect } from '@sveltejs/kit';
import { apiGet } from '$lib/server/api';
import { listTeams } from '$lib/server/graph-reads';
import type { ContextRowWithCounts, PublicSystemSettings, TeamRow } from '$lib/types';
import type { LayoutServerLoad } from './$types';

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
	//
	// `contexts` is `null` on a failed read, NOT `[]`. An empty list is a claim —
	// "there is nothing to filter by" in the vault filter bar's Context select,
	// "you belong to nothing" in the sidebar — and a fetch which never answered
	// cannot support either. Both consumers render the two apart.
	//
	// `teams` is the reader's memberships, and the nav uses it for exactly two
	// things: naming a group by its display name rather than its `+slug` ref, and
	// showing a team the reader belongs to that holds no readable place. It is
	// deliberately NOT what the grouping is keyed on — `/api/contexts` is scoped
	// by `context_visible_to` while `/api/teams` returns only teams the caller is
	// a member of, so a team-owned context can be readable without membership.
	// A failed teams read therefore degrades labels and drops empty groups; it
	// can never drop a place.
	const [contexts, settings, teams] = await Promise.all([
		apiGet<ContextRowWithCounts[]>('/api/contexts', locals.accessToken!).catch(() => null),
		apiGet<PublicSystemSettings>('/api/access/settings', locals.accessToken!).catch(() => null),
		listTeams(locals.accessToken!).catch((): TeamRow[] | null => null),
	]);

	return {
		user: locals.user,
		profile: locals.profile,
		entitlements: locals.entitlements,
		contexts,
		teams,
		instanceName: settings?.instance_name ?? null,
	};
};
