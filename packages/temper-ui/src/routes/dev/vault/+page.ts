// +page.ts — dev-only vault render harness loader.
//
// Renders the REAL resource-detail page, the real governed-families list and the real filter
// bar against committed, personal-data-free fixtures — no auth, no server reads — so the
// artifact surfaces this branch shipped can be LOOKED AT before merge. Same gap the graph
// harness closes for the graph surface: Vercel previews cannot carry Auth0, so authenticated
// UI is otherwise observable only in prod, post-merge.
//
// 404s outside `dev` so the route is inert in any deployed build.

import { error } from '@sveltejs/kit';
import { dev } from '$app/environment';
import type { PageLoad } from './$types';

export const load: PageLoad = async () => {
	if (!dev) throw error(404, 'Not found');
	return {};
};
