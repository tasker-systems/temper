/**
 * Dashboard server load — fetches real data from the gated API surface.
 *
 * The whole point of pulling these now (rather than leaving the dashboard as
 * a placeholder) is to verify that the bearer-token pass-through from
 * hooks.server.ts → apiGet() → temper-api → require_system_access middleware
 * actually works end-to-end. If anything is misconfigured (token format,
 * audience, system_access enforcement), the user sees the failure mode
 * here on the first authed page they hit, not buried in some later feature.
 *
 * `parent()` exposes the layout's `user` / `profile` / `entitlements`
 * without re-fetching them.
 */

import type { PageServerLoad } from './$types';
import { apiGet, ApiError } from '$lib/server/api';
import type { ResourceRow, ContextRow } from '$lib/types';

export const load: PageServerLoad = async ({ locals, parent }) => {
	const layoutData = await parent();
	const accessToken = locals.accessToken!;

	// Fetch in parallel — we want to surface either succeeding even if the
	// other fails, since this is the first place we're exercising
	// pass-through auth and partial data is more useful than a hard error.
	const [recentResources, contexts] = await Promise.all([
		apiGet<ResourceRow[]>('/api/resources?limit=5', accessToken).catch((err: unknown) => {
			console.error('Dashboard: failed to load recent resources', err);
			return null;
		}),
		apiGet<ContextRow[]>('/api/contexts', accessToken).catch((err: unknown) => {
			console.error('Dashboard: failed to load contexts', err);
			return null;
		})
	]);

	// Capture any errors so the page can show a banner explaining what failed
	// (rather than silently rendering empty lists).
	const apiErrors: string[] = [];
	if (recentResources === null) apiErrors.push('Could not load recent resources from the API.');
	if (contexts === null) apiErrors.push('Could not load contexts from the API.');

	return {
		...layoutData,
		recentResources: recentResources ?? [],
		contexts: contexts ?? [],
		apiErrors
	};
};
