// +page.ts — dev-only graph render harness loader.
//
// Renders the real `GraphPage` against committed, personal-data-free fixtures so the successor
// surface can be LOOKED AT in-branch. Vercel previews cannot carry Auth0, so authenticated UI is
// otherwise observable only in prod post-merge — the gap `/dev/atlas` used to close for the
// predecessor, and which nothing has closed since Beat D deleted it.
//
// 404s outside `dev` so the route is inert in any deployed build.

import { error } from '@sveltejs/kit';
import { dev } from '$app/environment';
import type { HarnessBundle } from '$lib/graph/harness';
import bundle from '../../../test/fixtures/graph-harness.json';
import type { PageLoad } from './$types';

export const load: PageLoad = async () => {
	if (!dev) throw error(404, 'Not found');
	return { bundle: bundle as unknown as HarnessBundle };
};
