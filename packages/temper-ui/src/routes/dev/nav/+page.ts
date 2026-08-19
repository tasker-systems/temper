// +page.ts — dev-only nav render harness loader.
//
// Renders the real `Sidebar` against hand-shaped fixtures so the grouping, the
// per-group collapse, and the three availability states can be seen in-branch.
// Vercel previews cannot carry Auth0, so the authenticated nav was otherwise
// only observable in prod post-merge — the same gap `/dev/atlas` exists to close.
//
// Fixtures are inline rather than captured: the nav reads two small, fully
// public-shaped lists (`ContextRowWithCounts`, `TeamRow`), so there is nothing a
// capture would tell us that the type does not, and nothing to sanitize.
//
// 404s outside `dev` so the route is inert in any deployed build.

import { error } from '@sveltejs/kit';
import { dev } from '$app/environment';
import type { PageLoad } from './$types';

export const load: PageLoad = async () => {
	if (!dev) throw error(404, 'Not found');
	return {};
};
