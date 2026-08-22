// +page.ts — dev-only analysis render harness loader.
//
// The receiver half. `AnalysisPage` is where `displaced-structure-remains-reachable` is paid, and
// it is the one surface whose fixture already existed: `graph-analysis-anchors.json` is an
// UNTRIMMED capture of all three shapes the door receives, including an anchor that has never
// materialized a region.
//
// 404s outside `dev` so the route is inert in any deployed build.

import { error } from '@sveltejs/kit';
import { dev } from '$app/environment';
import type { AnalysisBundle } from '$lib/graph/harness';
import bundle from '../../../test/fixtures/graph-analysis-anchors.json';
import type { PageLoad } from './$types';

export const load: PageLoad = async () => {
	if (!dev) throw error(404, 'Not found');
	return { bundle: bundle as unknown as AnalysisBundle };
};
